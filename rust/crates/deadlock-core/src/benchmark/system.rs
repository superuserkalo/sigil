//! The machine and the game settings a run ran on. Read only, no tool is run: Windows reads the
//! registry and `GlobalMemoryStatusEx`, Linux reads `/proc` and `/sys`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Machine {
    pub os: String,
    pub cpu: Option<String>,
    pub threads: Option<u32>,
    pub ram_mb: Option<u64>,
    pub gpu: Option<String>,
    pub driver: Option<String>,
    pub driver_date: Option<String>,
}

impl Machine {
    /// "GPU · driver", for history markers: a change here is a driver update.
    pub fn gpu_line(&self) -> String {
        match (&self.gpu, &self.driver) {
            (Some(gpu), Some(driver)) => format!("{gpu} · {driver}"),
            (Some(gpu), None) => gpu.clone(),
            _ => String::new(),
        }
    }
}

/// The game side of a run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// `ClientVersion` from `steam.inf`.
    pub build: Option<String>,
    /// Every `setting.*` line of the video.txt the game reads.
    pub video: BTreeMap<String, String>,
    pub video_path: Option<PathBuf>,
    /// The Performance config that is on, by id; None is stock.
    pub performance_config: Option<String>,
    pub profile: Option<String>,
    /// Enabled mods as (slot, name).
    pub mods: Vec<(u16, String)>,
}

impl Settings {
    pub fn resolution(&self) -> Option<String> {
        let w = self.video.get("defaultres")?;
        let h = self.video.get("defaultresheight")?;
        Some(format!("{w} x {h}"))
    }

    /// Fullscreen, borderless or windowed, from video.txt.
    pub fn window_mode(&self) -> Option<&'static str> {
        let on = |k: &str| self.video.get(k).map(|v| v.trim() == "1");
        match (on("fullscreen")?, on("nowindowborder").unwrap_or(false)) {
            (true, _) => Some("Fullscreen"),
            (false, true) => Some("Borderless"),
            (false, false) => Some("Windowed"),
        }
    }

    /// A short hash of the video settings: same hash, same settings.
    pub fn video_fingerprint(&self) -> String {
        fingerprint(self.video.iter().map(|(k, v)| format!("{k}={v}")))
    }

    /// A short hash of the enabled mod set.
    pub fn mods_fingerprint(&self) -> String {
        fingerprint(self.mods.iter().map(|(slot, name)| format!("{slot}:{name}")))
    }
}

fn fingerprint(lines: impl Iterator<Item = String>) -> String {
    let mut hash = Sha256::new();
    for line in lines {
        hash.update(line.as_bytes());
        hash.update(b"\n");
    }
    hash.finalize().iter().take(6).map(|b| format!("{b:02x}")).collect()
}

/// Every `"setting.<key>" "<value>"` line of a video.txt.
pub fn video_settings(text: &str) -> BTreeMap<String, String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r#""setting\.([A-Za-z_]\w*)"\s+"([^"\r\n]*)""#).expect("video"));
    re.captures_iter(text).map(|c| (c[1].to_string(), c[2].to_string())).collect()
}

/// Video lines that differ between two runs: (key, a, b).
pub fn video_diff(a: &Settings, b: &Settings) -> Vec<(String, Option<String>, Option<String>)> {
    let mut keys: Vec<&String> = a.video.keys().chain(b.video.keys()).collect();
    keys.sort();
    keys.dedup();
    keys.into_iter()
        .filter_map(|k| {
            let (x, y) = (a.video.get(k), b.video.get(k));
            (x != y).then(|| (k.clone(), x.cloned(), y.cloned()))
        })
        .collect()
}

/// The game side for `game_root`: build, video.txt, the Performance config. Profile and mods
/// come from the caller, which knows the library.
pub fn read_settings(game_root: &Path, steam_roots: &[PathBuf]) -> Settings {
    let video_path = crate::performance::video::locate(game_root, steam_roots);
    let video = video_path
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| video_settings(&t))
        .unwrap_or_default();
    let performance_config = std::fs::read_to_string(crate::performance::gameinfo_path(game_root))
        .ok()
        .and_then(|text| match crate::performance::applied(&text) {
            crate::performance::Applied::Config { id, .. } => Some(id),
            crate::performance::Applied::Foreign => Some("grimoire".to_string()),
            crate::performance::Applied::None => None,
        });
    Settings {
        build: crate::host::read_build(game_root),
        video,
        video_path,
        performance_config,
        profile: None,
        mods: Vec::new(),
    }
}

/// This machine.
pub fn read_machine() -> Machine {
    let threads = std::thread::available_parallelism().ok().map(|n| n.get() as u32);
    let mut machine = platform::read();
    machine.threads = machine.threads.or(threads);
    machine
}

#[cfg(windows)]
mod platform {
    use super::Machine;
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW,
    };
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn reg(root: HKEY, sub: &str, name: &str) -> Option<String> {
        let sub = wide(sub);
        let name = wide(name);
        let mut buffer = vec![0u16; 512];
        let mut bytes = (buffer.len() * 2) as u32;
        // SAFETY: both strings are NUL-terminated and the buffer length is passed in bytes.
        let status = unsafe {
            RegGetValueW(
                root,
                sub.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buffer.as_mut_ptr().cast(),
                &mut bytes,
            )
        };
        if status != ERROR_SUCCESS {
            return None;
        }
        let len = (bytes as usize / 2).min(buffer.len());
        let text = String::from_utf16_lossy(&buffer[..len]);
        let text = text.trim_end_matches('\0').trim().to_string();
        (!text.is_empty()).then_some(text)
    }

    const ADAPTERS: &str = r"SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}";

    pub fn read() -> Machine {
        let cpu = reg(HKEY_LOCAL_MACHINE, r"HARDWARE\DESCRIPTION\System\CentralProcessor\0", "ProcessorNameString");
        let nt = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
        let build = reg(HKEY_LOCAL_MACHINE, nt, "CurrentBuild");
        let product = reg(HKEY_LOCAL_MACHINE, nt, "ProductName").unwrap_or_else(|| "Windows".into());
        // Windows 11 still says "Windows 10" in ProductName; the build tells them apart.
        let product = match build.as_deref().and_then(|b| b.parse::<u32>().ok()) {
            Some(b) if b >= 22000 => product.replace("Windows 10", "Windows 11"),
            _ => product,
        };
        let display = reg(HKEY_LOCAL_MACHINE, nt, "DisplayVersion");
        let os = [Some(product), display, build.map(|b| format!("build {b}"))]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ");
        // The display adapters: a discrete card over integrated graphics, never the basic driver.
        let mut adapters: Vec<(String, Option<String>, Option<String>)> = Vec::new();
        for index in 0..10 {
            let key = format!(r"{ADAPTERS}\{index:04}");
            let Some(name) = reg(HKEY_LOCAL_MACHINE, &key, "DriverDesc") else { continue };
            if name.contains("Basic Display") || name.contains("Basic Render") || name.contains("Virtual") {
                continue;
            }
            adapters.push((name, reg(HKEY_LOCAL_MACHINE, &key, "DriverVersion"), reg(HKEY_LOCAL_MACHINE, &key, "DriverDate")));
        }
        let rank = |name: &str| {
            let n = name.to_ascii_lowercase();
            if n.contains("nvidia") || n.contains("geforce") || n.contains("radeon rx") || n.contains("arc") {
                0
            } else if n.contains("radeon") || n.contains("amd") {
                1
            } else {
                2
            }
        };
        adapters.sort_by_key(|a| rank(&a.0));
        let gpu = adapters.into_iter().next();
        let mut status = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..unsafe { std::mem::zeroed() } };
        // SAFETY: dwLength is set as the API asks; the struct is plain data.
        let ram_mb = (unsafe { GlobalMemoryStatusEx(&mut status) } != 0).then(|| status.ullTotalPhys / (1024 * 1024));
        Machine {
            os,
            cpu,
            threads: None,
            ram_mb,
            gpu: gpu.as_ref().map(|g| g.0.clone()),
            driver: gpu.as_ref().and_then(|g| g.1.clone()),
            driver_date: gpu.and_then(|g| g.2),
        }
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::Machine;

    fn vendor_name(id: &str) -> &'static str {
        match id.trim().trim_start_matches("0x") {
            "10de" => "NVIDIA",
            "1002" => "AMD",
            "8086" => "Intel",
            _ => "GPU",
        }
    }

    pub fn read() -> Machine {
        let cpu = std::fs::read_to_string("/proc/cpuinfo").ok().and_then(|t| {
            t.lines()
                .find(|l| l.starts_with("model name"))
                .and_then(|l| l.split(':').nth(1))
                .map(|s| s.trim().to_string())
        });
        let ram_mb = std::fs::read_to_string("/proc/meminfo").ok().and_then(|t| {
            t.lines()
                .find(|l| l.starts_with("MemTotal:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|kb| kb.parse::<u64>().ok())
                .map(|kb| kb / 1024)
        });
        let mut gpu = None;
        let mut driver = None;
        if let Ok(entries) = std::fs::read_dir("/sys/class/drm") {
            let mut cards: Vec<_> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.file_name().is_some_and(|n| { let n = n.to_string_lossy(); n.starts_with("card") && !n.contains('-') }))
                .collect();
            cards.sort();
            for card in cards {
                let device = card.join("device");
                let Ok(vendor) = std::fs::read_to_string(device.join("vendor")) else { continue };
                let id = std::fs::read_to_string(device.join("device")).unwrap_or_default();
                let name = vendor_name(&vendor);
                gpu = Some(format!("{name} {}", id.trim()));
                driver = std::fs::read_link(device.join("driver"))
                    .ok()
                    .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
                if name == "NVIDIA"
                    && let Ok(v) = std::fs::read_to_string("/proc/driver/nvidia/version")
                {
                    driver = v.split_whitespace().skip_while(|w| !w.contains("Module")).nth(1).map(str::to_string).or(driver);
                }
                if name != "Intel" {
                    break;
                }
            }
        }
        let os = std::fs::read_to_string("/etc/os-release")
            .ok()
            .and_then(|t| t.lines().find_map(|l| l.strip_prefix("PRETTY_NAME=").map(|v| v.trim_matches('"').to_string())))
            .unwrap_or_else(|| "Linux".into());
        Machine { os, cpu, threads: None, ram_mb, gpu, driver, driver_date: None }
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod platform {
    use super::Machine;

    pub fn read() -> Machine {
        Machine { os: std::env::consts::OS.to_string(), ..Machine::default() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_settings_read_every_setting_line() {
        let text = "\"video.cfg\"\n{\n\t\"Version\"\t\t\"16\"\n\t\"setting.defaultres\"\t\t\"2560\"\n\t\"setting.defaultresheight\"\t\t\"1440\"\n\t\"setting.fullscreen\"\t\t\"0\"\n\t\"setting.nowindowborder\"\t\t\"1\"\n\t\"setting.r_citadel_fsr\"\t\t\"2\"\n}\n";
        let s = Settings { video: video_settings(text), ..Settings::default() };
        assert_eq!(s.video.len(), 5);
        assert_eq!(s.resolution().as_deref(), Some("2560 x 1440"));
        assert_eq!(s.window_mode(), Some("Borderless"));
        assert_eq!(s.video_fingerprint().len(), 12);
    }

    #[test]
    fn the_diff_lists_changed_lines_only() {
        let a = Settings { video: video_settings("\"setting.a\" \"1\" \"setting.b\" \"2\""), ..Settings::default() };
        let b = Settings { video: video_settings("\"setting.a\" \"1\" \"setting.b\" \"3\" \"setting.c\" \"x\""), ..Settings::default() };
        let diff = video_diff(&a, &b);
        assert_eq!(diff, vec![("b".into(), Some("2".into()), Some("3".into())), ("c".into(), None, Some("x".into()))]);
        assert_ne!(a.video_fingerprint(), b.video_fingerprint());
    }

    #[test]
    fn the_machine_reads_without_failing() {
        let machine = read_machine();
        assert!(!machine.os.is_empty());
        assert!(machine.threads.unwrap_or(1) >= 1);
    }
}
