use std::ffi::OsString;
use std::fs;
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};

use super::*;

fn scratch(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("sigil-host-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

/// Minimal stored ZIP writer (as in `install.rs` tests), so hostile names survive into a fixture.
fn stored_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut output = Vec::new();
    let mut central = Vec::new();
    let mut offsets = Vec::new();
    for (name, data) in entries {
        let crc = crc32fast::hash(data);
        offsets.push(output.len() as u32);
        output.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
        output.extend_from_slice(&20_u16.to_le_bytes());
        output.extend_from_slice(&[0u8; 8]);
        output.extend_from_slice(&crc.to_le_bytes());
        output.extend_from_slice(&(data.len() as u32).to_le_bytes());
        output.extend_from_slice(&(data.len() as u32).to_le_bytes());
        output.extend_from_slice(&(name.len() as u16).to_le_bytes());
        output.extend_from_slice(&0_u16.to_le_bytes());
        output.extend_from_slice(name.as_bytes());
        output.extend_from_slice(data);
    }
    let central_start = output.len() as u32;
    for (index, (name, data)) in entries.iter().enumerate() {
        let crc = crc32fast::hash(data);
        central.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
        central.extend_from_slice(&((3_u16 << 8) | 20).to_le_bytes());
        central.extend_from_slice(&20_u16.to_le_bytes());
        central.extend_from_slice(&[0u8; 8]);
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend_from_slice(&[0u8; 8]);
        central.extend_from_slice(&(0o100644_u32 << 16).to_le_bytes());
        central.extend_from_slice(&offsets[index].to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    output.extend_from_slice(&central);
    output.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
    output.extend_from_slice(&[0u8; 4]);
    output.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    output.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    output.extend_from_slice(&(central.len() as u32).to_le_bytes());
    output.extend_from_slice(&central_start.to_le_bytes());
    output.extend_from_slice(&0_u16.to_le_bytes());
    output
}

fn args_of(command: &std::process::Command) -> Vec<String> {
    command.get_args().map(|a| a.to_string_lossy().into_owned()).collect()
}

// --- config ---------------------------------------------------------------------------------

#[test]
fn settings_round_trip_through_app_data() {
    let data = scratch("settings");
    assert_eq!(load_settings(&data).unwrap(), HostSettings::default());
    let settings = HostSettings {
        server_dir: Some(PathBuf::from("D:/deadworks-server")),
        steamcmd_dir: Some(PathBuf::from("D:/steamcmd")),
        steam_user: "player_one".into(),
        server: ServerConfig {
            name: "Friday Brawl".into(),
            map: "dl_midtown".into(),
            port: 27100,
            password: "letmein".into(),
            max_players: Some(12),
            extra: "-ip 192.168.1.20".into(),
            fastdl_port: Some(27200),
        },
    };
    save_settings(&data, &settings).unwrap();
    assert!(settings_path(&data).ends_with("host/settings.json"));
    assert_eq!(load_settings(&data).unwrap(), settings);
    let text = fs::read_to_string(settings_path(&data)).unwrap();
    assert!(!text.to_lowercase().contains("steampassword"));
    let _ = fs::remove_dir_all(&data);
}

#[test]
fn defaults_follow_the_deadworks_guide() {
    let config = ServerConfig::default();
    assert_eq!(config.port, 27067);
    assert_eq!(config.map, "dl_midtown");
    config.validate().unwrap();
}

#[test]
fn config_rejects_values_the_cfg_cannot_carry() {
    let base = ServerConfig::default();
    for bad in [
        ServerConfig { name: "a\"b".into(), ..base.clone() },
        ServerConfig { name: "x; quit".into(), ..base.clone() },
        ServerConfig { name: " ".into(), ..base.clone() },
        ServerConfig { map: "dl_midtown; quit".into(), ..base.clone() },
        ServerConfig { map: "../x".into(), ..base.clone() },
        ServerConfig { port: 80, ..base.clone() },
        ServerConfig { password: "a b".into(), ..base.clone() },
        ServerConfig { password: "a\nrcon_password x".into(), ..base.clone() },
        ServerConfig { max_players: Some(0), ..base.clone() },
        ServerConfig { extra: "-ip 1\n+quit".into(), ..base.clone() },
    ] {
        assert!(bad.validate().is_err(), "{bad:?}");
    }
}

#[test]
fn launch_line_is_exact_and_never_holds_the_password_or_name() {
    let config = ServerConfig {
        name: "My Server".into(),
        password: "hunter2".into(),
        ..ServerConfig::default()
    };
    let expected: Vec<&str> = vec![
        "-dedicated", "-console", "-dev", "-insecure", "-allow_no_lobby_connect",
        "+tv_citadel_auto_record", "0", "+spec_replay_enable", "0", "+tv_enable", "0",
        "+citadel_upload_replay_enabled", "0", "+hostport", "27067", "+exec", "sigil_server.cfg",
        "+map", "dl_midtown",
    ];
    assert_eq!(config.launch_args(), expected);
    let command = server_command(Path::new("/srv/dw"), &config).unwrap();
    assert_eq!(args_of(&command), expected);
    assert!(command.get_program().to_string_lossy().ends_with("deadworks.exe"));
    assert!(command.get_current_dir().unwrap().ends_with("game/bin/win64"));
    assert!(!args_of(&command).iter().any(|a| a.contains("hunter2") || a.contains("My Server")));

    let cfg = config.cfg_text();
    assert!(cfg.contains("hostname \"My Server\"\n"));
    assert!(cfg.contains("sv_password \"hunter2\"\n"));
}

#[test]
fn launch_line_adds_max_players_and_extra_options() {
    let config = ServerConfig {
        port: 27100,
        map: "dl_streets".into(),
        max_players: Some(8),
        extra: "  -ip 10.0.0.5   +sv_cheats 0 ".into(),
        ..ServerConfig::default()
    };
    let args = config.launch_args();
    let tail: Vec<&str> = args[13..].iter().map(String::as_str).collect();
    assert_eq!(
        tail,
        ["+hostport", "27100", "-maxplayers", "8", "+exec", "sigil_server.cfg", "+map", "dl_streets", "-ip", "10.0.0.5", "+sv_cheats", "0"]
    );
}

// --- SteamCMD -------------------------------------------------------------------------------

#[test]
fn steamcmd_command_is_exact_and_carries_no_password() {
    let server = Path::new("/srv/deadworks-server");
    let command = steamcmd_update_command(Path::new("/tools/steamcmd/steamcmd.exe"), server, "player_one").unwrap();
    assert_eq!(
        args_of(&command),
        ["+force_install_dir", "/srv/deadworks-server", "+login", "player_one", "+app_update", "1422450", "validate", "+quit"]
    );
    assert_eq!(command.get_current_dir(), Some(Path::new("/tools/steamcmd")));
    assert!(command.get_envs().next().is_none());
    // Exactly one value after +login: the account name. SteamCMD prompts for the rest.
    let args = steamcmd_args(server, "player_one").unwrap();
    let login = args.iter().position(|a| a == &OsString::from("+login")).unwrap();
    assert_eq!(args[login + 2], OsString::from("+app_update"));
}

#[test]
fn steamcmd_refuses_anything_but_an_account_name() {
    let server = Path::new("/srv/dw");
    for user in ["", "name password", "name\tpw", "a+quit", "user;x", "\"x\"", &"a".repeat(65)] {
        assert!(steamcmd_args(server, user).is_err(), "{user:?}");
    }
    assert!(valid_steam_user("Player_01"));
}

#[test]
fn server_folder_must_stay_out_of_steam_and_the_game() {
    let game = Path::new("/games/Deadlock");
    assert!(check_server_dir(Path::new("/srv/dw"), Some(game)).is_ok());
    assert!(check_server_dir(Path::new("/games/Deadlock/server"), Some(game)).is_err());
    assert!(check_server_dir(Path::new("/games"), Some(game)).is_err());
    assert!(check_server_dir(Path::new("/Steam/steamapps/common/dw"), None).is_err());
    assert!(check_server_dir(Path::new("relative/dw"), None).is_err());
    assert!(check_server_dir(Path::new("/srv/../dw"), None).is_err());
    assert!(steamcmd_update_command(Path::new("/s/steamcmd.exe"), Path::new("/Steam/SteamApps/common/x"), "u").is_err());
}

#[test]
fn steamcmd_lines_parse() {
    assert_eq!(
        parse_steamcmd_line(" Update state (0x61) downloading, progress: 45.20 (1234 / 5678)"),
        Some(SteamCmdEvent::Progress { percent: 45.2, state: "downloading".into() })
    );
    assert_eq!(
        parse_steamcmd_line("Success! App '1422450' fully installed."),
        Some(SteamCmdEvent::Installed)
    );
    assert!(matches!(
        parse_steamcmd_line("ERROR! Failed to install app '1422450' (No subscription)"),
        Some(SteamCmdEvent::Failed { .. })
    ));
    assert_eq!(parse_steamcmd_line("Logging in user 'player_one' to Steam Public..."), None);
}

#[test]
fn steamcmd_log_tail_skips_password_lines() {
    let dir = scratch("steamcmd-log");
    fs::create_dir_all(dir.join("logs")).unwrap();
    fs::write(
        dir.join("logs/console_log.txt"),
        "one\r\npassword: \r\ntwo\n\nthree\nCached credentials not found. password needed\n",
    )
    .unwrap();
    assert_eq!(steamcmd_log_lines(&dir, 2), ["two", "three"]);
    assert_eq!(steamcmd_log_lines(&dir, 10), ["one", "two", "three"]);
    assert!(steamcmd_log_lines(&dir.join("missing"), 10).is_empty());
    let _ = fs::remove_dir_all(&dir);
}

// --- prerequisites --------------------------------------------------------------------------

/// Recorded `dotnet --list-runtimes` from a Windows machine with .NET 8 and 10.
const DOTNET_RECORDED: &str = "Microsoft.AspNetCore.App 8.0.11 [C:\\Program Files\\dotnet\\shared\\Microsoft.AspNetCore.App]\r
Microsoft.NETCore.App 8.0.11 [C:\\Program Files\\dotnet\\shared\\Microsoft.NETCore.App]\r
Microsoft.NETCore.App 10.0.0 [C:\\Program Files\\dotnet\\shared\\Microsoft.NETCore.App]\r
Microsoft.WindowsDesktop.App 10.0.0 [C:\\Program Files\\dotnet\\shared\\Microsoft.WindowsDesktop.App]\r
";

#[test]
fn dotnet_runtimes_parse() {
    let status = parse_dotnet_runtimes(DOTNET_RECORDED);
    assert_eq!(status.versions, ["8.0.11", "10.0.0"]);
    assert!(status.ok);
    let old = parse_dotnet_runtimes("Microsoft.NETCore.App 8.0.11 [C:\\x]\nMicrosoft.AspNetCore.App 10.0.0 [C:\\y]\n");
    assert!(!old.ok, "ASP.NET 10 alone is not the .NET 10 runtime");
    assert!(parse_dotnet_runtimes("Microsoft.NETCore.App 10.0.0-rc.2.25502.107 [C:\\x]").ok);
    assert!(!parse_dotnet_runtimes("").ok);
    assert!(!parse_dotnet_runtimes("Microsoft.NETCore.App 100.0.0 [x]").ok);
}

#[test]
fn steam_inf_parses_the_client_version() {
    let recorded = "ClientVersion=6123\r\nServerVersion=6123\r\nPatchVersion=1.0.0.0\r\nProductName=citadel\r\nappID=1422450\r\n";
    assert_eq!(parse_steam_inf(recorded).as_deref(), Some("6123"));
    assert_eq!(parse_steam_inf("PatchVersion=1\n"), None);
    assert_eq!(parse_steam_inf("ClientVersion=\n"), None);
}

#[test]
fn update_advice_compares_builds() {
    let root = scratch("builds");
    let server = root.join("server");
    let game = root.join("game-install");
    let write = |dir: &Path, build: &str| {
        fs::create_dir_all(dir.join("game/citadel")).unwrap();
        fs::write(dir.join("game/citadel/steam.inf"), format!("ClientVersion={build}\n")).unwrap();
    };
    let missing = update_advice(&server, Some(&game));
    assert!(missing.needs_update && missing.server_build.is_none());

    write(&server, "6100");
    write(&game, "6123");
    let advice = update_advice(&server, Some(&game));
    assert!(advice.needs_update);
    assert!(advice.message.contains("older than your game"), "{}", advice.message);

    write(&server, "6123");
    assert!(!update_advice(&server, Some(&game)).needs_update);
    assert_eq!(read_build(&server).as_deref(), Some("6123"));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn windows_only_calls_refuse_elsewhere() {
    if hosting_supported() {
        return;
    }
    let dir = scratch("not-windows");
    let message = |r: crate::error::Result<()>| r.unwrap_err().to_string();
    assert_eq!(message(prerequisites(&dir, None).map(|_| ())), HOSTING_NEEDS_WINDOWS);
    assert_eq!(message(install_steamcmd(&dir, true).map(|_| ())), HOSTING_NEEDS_WINDOWS);
    assert_eq!(message(install_deadworks(&dir, true).map(|_| ())), HOSTING_NEEDS_WINDOWS);
    assert_eq!(message(start_server(&dir, &ServerConfig::default(), true).map(|_| ())), HOSTING_NEEDS_WINDOWS);
    assert_eq!(message(firewall_rule_status(27067, Some(27068)).map(|_| ())), HOSTING_NEEDS_WINDOWS);
    assert_eq!(message(add_firewall_rule(27067, Some(27068), true).map(|_| ())), HOSTING_NEEDS_WINDOWS);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn every_action_needs_approval() {
    let dir = scratch("approve");
    for result in [
        install_steamcmd(&dir, false).map(|_| ()),
        install_deadworks(&dir, false).map(|_| ()),
        start_server(&dir, &ServerConfig::default(), false).map(|_| ()),
        add_firewall_rule(27067, Some(27068), false).map(|_| ()),
        remove_firewall_rule(27067, Some(27068), false).map(|_| ()),
        set_plugin(&dir, "X", true, false).map(|_| ()),
        start_steamcmd_update(&dir.join("steamcmd.exe"), &dir, "u", false).map(|_| ()),
    ] {
        assert!(result.unwrap_err().to_string().contains("not approved"));
    }
    assert!(fs::read_dir(&dir).unwrap().next().is_none(), "nothing was written");
    let _ = fs::remove_dir_all(&dir);
}

// --- Deadworks install ----------------------------------------------------------------------

#[test]
fn pinned_release_constants_agree() {
    assert!(DEADWORKS_URL.ends_with(&format!("/{DEADWORKS_VERSION}/deadworks-{DEADWORKS_VERSION}.zip")));
    assert!(DEADWORKS_URL.starts_with("https://github.com/Deadworks-net/deadworks/releases/download/"));
    assert_eq!(DEADWORKS_SHA256.len(), 64);
    assert_eq!(STEAMCMD_SHA256.len(), 64);
    assert!(STEAMCMD_URL.starts_with("https://steamcdn-a.akamaihd.net/"));
    assert!(setup::check_download_url(DEADWORKS_URL, &["github.com"]).is_ok());
    assert!(setup::check_download_url("http://github.com/x", &["github.com"]).is_err());
    assert!(setup::check_download_url("https://github.com.evil.io/x", &["github.com"]).is_err());
    assert!(setup::check_download_url("https://user@github.com/x", &["github.com"]).is_err());
}

#[test]
fn deadworks_zip_merges_over_the_server_and_keeps_plugins() {
    let root = scratch("dw-install");
    let server = root.join("server");
    fs::create_dir_all(server.join("game/citadel")).unwrap();
    fs::create_dir_all(plugins_dir(&server)).unwrap();
    fs::write(plugins_dir(&server).join("Mine.dll"), b"mine").unwrap();
    fs::write(server.join("game/citadel/steam.inf"), "ClientVersion=1\n").unwrap();

    let old = root.join("old.zip");
    fs::write(&old, stored_zip(&[
        ("game/bin/win64/deadworks.exe", b"exe-old"),
        ("game/bin/win64/managed/Old.dll", b"old"),
    ]))
    .unwrap();
    install_deadworks_from_zip(&old, &server, "v0.6.3", true).unwrap();
    assert_eq!(deadworks_installed(&server).unwrap().version.as_deref(), Some("v0.6.3"));
    assert!(!deadworks_installed(&server).unwrap().is_pinned);

    let new = root.join("new.zip");
    fs::write(&new, stored_zip(&[
        ("game/", b""),
        ("game/bin/win64/deadworks.exe", b"exe-new"),
        ("game/bin/win64/managed/plugins/Bundled.dll", b"would clobber"),
        ("game/citadel/cfg/deadworks_mem.jsonc", b"{}"),
    ]))
    .unwrap();
    let report = install_deadworks_from_zip(&new, &server, DEADWORKS_VERSION, true).unwrap();
    assert_eq!(report.removed, ["game/bin/win64/managed/Old.dll"]);
    assert_eq!(fs::read(deadworks_exe(&server)).unwrap(), b"exe-new");
    assert_eq!(fs::read(plugins_dir(&server).join("Mine.dll")).unwrap(), b"mine");
    assert!(!plugins_dir(&server).join("Bundled.dll").exists());
    assert_eq!(fs::read_to_string(server.join("game/citadel/steam.inf")).unwrap(), "ClientVersion=1\n");
    assert!(deadworks_installed(&server).unwrap().is_pinned);
    assert!(!server.join(".sigil-extract").exists());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn deadworks_zip_that_escapes_is_refused() {
    let root = scratch("dw-escape");
    let server = root.join("server");
    fs::create_dir_all(&server).unwrap();
    for (index, name) in ["game/../../evil.txt", "/abs/evil.txt", "C:/evil.txt", "game\\..\\evil.txt"].iter().enumerate() {
        let zip = root.join(format!("bad{index}.zip"));
        fs::write(&zip, stored_zip(&[("game/bin/win64/deadworks.exe", b"exe"), (name, b"x")])).unwrap();
        assert!(install_deadworks_from_zip(&zip, &server, "v0", true).is_err(), "{name}");
        assert!(!deadworks_exe(&server).exists(), "{name}: nothing lands");
    }
    let outside = root.join("outside.zip");
    fs::write(&outside, stored_zip(&[("game/bin/win64/deadworks.exe", b"exe"), ("readme.txt", b"x")])).unwrap();
    assert!(install_deadworks_from_zip(&outside, &server, "v0", true).is_err());
    assert!(!root.join("evil.txt").exists());
    assert!(!deadworks_exe(&server).exists());
    let _ = fs::remove_dir_all(&root);
}

// --- plugins --------------------------------------------------------------------------------

#[test]
fn plugins_move_between_on_and_off() {
    let server = scratch("plugins");
    fs::create_dir_all(plugins_dir(&server)).unwrap();
    fs::write(plugins_dir(&server).join("Deathmatch.dll"), b"dm").unwrap();
    fs::write(plugins_dir(&server).join("Deathmatch.deps.json"), b"{}").unwrap();
    fs::create_dir_all(plugins_off_dir(&server)).unwrap();
    fs::write(plugins_off_dir(&server).join("Tag.dll"), b"tag").unwrap();

    let listed = list_plugins(&server).unwrap();
    assert_eq!(listed.iter().map(|p| (p.name.as_str(), p.enabled)).collect::<Vec<_>>(), [("Deathmatch", true), ("Tag", false)]);

    let off = set_plugin(&server, "Deathmatch", false, true).unwrap();
    assert!(!off.enabled);
    assert!(plugins_off_dir(&server).join("Deathmatch.dll").is_file());
    assert!(!plugins_dir(&server).join("Deathmatch.dll").exists());

    let on = set_plugin(&server, "Tag.dll", true, true).unwrap();
    assert!(on.enabled && plugins_dir(&server).join("Tag.dll").is_file());
    // Asking for the state it is already in is a no-op.
    assert!(set_plugin(&server, "Tag", true, true).unwrap().enabled);

    for bad in ["../Tag", "x/Tag", "C:Tag", "", ".."] {
        assert!(set_plugin(&server, bad, true, true).is_err(), "{bad:?}");
    }
    assert!(set_plugin(&server, "Nope", true, true).is_err());
    let _ = fs::remove_dir_all(&server);
}

#[test]
fn plugins_list_is_empty_without_folders() {
    let server = scratch("plugins-empty");
    assert!(list_plugins(&server).unwrap().is_empty());
    let _ = fs::remove_dir_all(&server);
}

// --- reach ----------------------------------------------------------------------------------

#[test]
fn firewall_show_output_parses() {
    let name = firewall_rule_name("TCP", 27067);
    assert_eq!(name, "Sigil-Deadworks-TCP-27067");
    let recorded = "\r\nRule Name:                            Sigil-Deadworks-TCP-27067\r\n----------------------------------------------------------------------\r\nEnabled:                              Yes\r\nDirection:                            In\r\nLocalPort:                            27067\r\nAction:                               Allow\r\nOk.\r\n";
    assert!(parse_firewall_show(true, recorded, &name));
    assert!(!parse_firewall_show(false, "No rules match the specified criteria.\r\n", &name));
    assert!(!parse_firewall_show(true, recorded, "Sigil-Deadworks-TCP-2706"));
}

#[test]
fn firewall_script_elevates_once_for_the_missing_rules() {
    let status = FirewallStatus { tcp: true, udp: false, downloads: Some(true) };
    let script = firewall_script(27067, Some(27068), status, true);
    assert_eq!(
        script,
        "try { $p = Start-Process -FilePath cmd.exe -ArgumentList '/c netsh advfirewall firewall add rule name=Sigil-Deadworks-UDP-27067 dir=in action=allow protocol=UDP localport=27067' -Verb RunAs -WindowStyle Hidden -Wait -PassThru; exit $p.ExitCode } catch { exit 1223 }"
    );
    // Nothing there yet: all three rules in one elevated command, so one admin prompt.
    let all = firewall_script(27100, Some(27101), FirewallStatus::default(), true);
    assert_eq!(all.matches("-Verb RunAs").count(), 1);
    assert!(all.contains("protocol=TCP localport=27100 && netsh advfirewall firewall add rule name=Sigil-Deadworks-UDP-27100"));
    assert!(all.contains(
        "protocol=UDP localport=27100 && netsh advfirewall firewall add rule name=Sigil-Deadworks-Downloads-TCP-27101 dir=in action=allow protocol=TCP localport=27101'"
    ));
    let remove = firewall_script(27100, Some(27101), FirewallStatus { tcp: true, udp: true, downloads: Some(true) }, false);
    assert!(remove.contains("delete rule name=Sigil-Deadworks-TCP-27100 && netsh advfirewall firewall delete rule name=Sigil-Deadworks-UDP-27100 && netsh advfirewall firewall delete rule name=Sigil-Deadworks-Downloads-TCP-27101'"));
    // Without a download port only the game rules are named.
    assert!(!firewall_script(27100, None, FirewallStatus::default(), true).contains("Downloads"));
}

#[test]
fn firewall_status_counts_the_download_rule() {
    let game_only = FirewallStatus { tcp: true, udp: true, downloads: Some(false) };
    assert!(!game_only.open() && !game_only.closed());
    assert!(FirewallStatus { tcp: true, udp: true, downloads: Some(true) }.open());
    assert!(FirewallStatus { tcp: true, udp: true, downloads: None }.open());
    assert!(FirewallStatus { tcp: false, udp: false, downloads: Some(false) }.closed());
    assert!(!FirewallStatus { tcp: false, udp: false, downloads: Some(true) }.closed());
    assert_eq!(download_rule_name(27068), "Sigil-Deadworks-Downloads-TCP-27068");
}

#[test]
fn join_links() {
    let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 20));
    assert_eq!(join_url(ip, 27067).unwrap(), "steam://connect/192.168.1.20:27067");
    assert_eq!(console_connect(ip, 27067), "connect 192.168.1.20:27067");
    assert!(lan_ipv4s().iter().all(Ipv4Addr::is_private));
}

#[test]
fn console_buffer_is_bounded() {
    let buffer = LineBuffer::default();
    for index in 0..(MAX_LINES + 5) {
        buffer.push(&format!("line {index}\r\n"));
    }
    let lines = buffer.lines();
    assert_eq!(lines.len(), MAX_LINES);
    assert_eq!(lines[0], "line 5");
    assert_eq!(buffer.total(), (MAX_LINES + 5) as u64);
    buffer.push(&"é".repeat(5000));
    assert!(buffer.lines().last().unwrap().len() <= 4096);
}

#[test]
fn unsupported_build_is_spotted() {
    let lines = vec!["[Deadworks] Failed to find signature for CCitadelPlayerPawn".to_string()];
    assert!(unsupported_build(&lines));
    assert!(!unsupported_build(&["Server is hibernating".to_string()]));
}

// --- server content -------------------------------------------------------------------------

/// A single-file VPK named `<file_name>` holding `files`.
fn content_vpk(root: &Path, file_name: &str, files: &[(&str, &[u8])]) -> PathBuf {
    let staging = root.join(format!("stage-{file_name}"));
    for (path, data) in files {
        let target = staging.join(path);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(target, data).unwrap();
    }
    let out = root.join(format!("out-{file_name}"));
    let packed = crate::vpk_write::pack_vpk_single(&crate::vpk_write::PackOptions::new(staging, out, "content")).unwrap();
    let target = root.join(file_name);
    fs::rename(packed.dir_path, &target).unwrap();
    target
}

fn content_server(root: &Path) -> PathBuf {
    let server = root.join("server");
    fs::create_dir_all(server_maps_dir(&server)).unwrap();
    fs::write(server_maps_dir(&server).join("dl_midtown.vpk"), b"stock map").unwrap();
    server
}

#[test]
fn custom_map_and_addon_install_into_the_server_and_are_advertised() {
    let root = scratch("content");
    let server = content_server(&root);
    fs::create_dir_all(deadworks_config_path(&server).parent().unwrap()).unwrap();
    fs::write(
        deadworks_config_path(&server),
        "// admin settings\n{\n  \"admins\": [\"123\"], /* keep */\n  \"serverbrowser\": { \"extra_maps\": [\"dl_other\"], },\n}\n",
    )
    .unwrap();
    let map = content_vpk(&root, "Arena.vpk", &[("maps/dl_arena.vmap_c", b"vmap"), ("maps/dl_arena/world.vwrld_c", b"w")]);
    let addon = content_vpk(&root, "turbo.vpk", &[("scripts/turbo.vdata_c", b"t")]);

    let installed = install_server_content(&server, &[map.clone(), addon], true).unwrap();
    assert_eq!(installed, ["maps/dl_arena", "addons/turbo"]);
    assert!(server_maps_dir(&server).join("dl_arena.vpk").is_file());
    assert!(server_addons_dir(&server).join("turbo.vpk").is_file());

    let config: serde_json::Value =
        serde_json::from_str(&strip_jsonc(&fs::read_to_string(deadworks_config_path(&server)).unwrap())).unwrap();
    assert_eq!(config["admins"][0], "123");
    assert_eq!(config["serverbrowser"]["extra_maps"], serde_json::json!(["dl_other", "dl_arena"]));
    assert_eq!(config["serverbrowser"]["content_addons"], serde_json::json!(["turbo"]));
    let backups: Vec<_> = fs::read_dir(server.join(CONTENT_BACKUPS)).unwrap().flatten().collect();
    assert_eq!(backups.len(), 1);
    assert!(backups[0].path().join("deadworks.jsonc").is_file(), "old config backed up");

    let listed = list_server_content(&server).unwrap();
    let arena = listed.iter().find(|c| c.name == "dl_arena").unwrap();
    assert!(arena.ours && arena.advertised && arena.hash.len() == 16);
    let stock = listed.iter().find(|c| c.name == "dl_midtown").unwrap();
    assert!(!stock.ours && stock.hash.is_empty());
    assert_eq!(server_map_names(&server), ["dl_arena", "dl_midtown"]);

    // Reinstalling backs up the old copy; the stock map is never replaced.
    install_server_content(&server, &[map], true).unwrap();
    let stock_vpk = content_vpk(&root, "dl_midtown.vpk", &[("maps/dl_midtown.vmap_c", b"x")]);
    assert!(install_server_content(&server, &[stock_vpk], true).is_err());
    assert_eq!(fs::read(server_maps_dir(&server).join("dl_midtown.vpk")).unwrap(), b"stock map");

    // Download layout matches what Sigil's joiner requests (servers::content_url).
    let out = default_fastdl_dir(&server);
    let files = build_fastdl(&server, &out, true).unwrap();
    let hash = &crate::servers::file_sha256(&server_maps_dir(&server).join("dl_arena.vpk")).unwrap()[..16];
    let entry = crate::servers::ContentEntry {
        kind: crate::servers::ContentKind::Map,
        name: "dl_arena".into(),
        hash: hash.to_string(),
        size: None,
    };
    let url = crate::servers::content_url("https://dl.example.com/x", &entry, "zst");
    let relative = url.strip_prefix("https://dl.example.com/x/").unwrap();
    assert!(files.contains(&out.join(relative)), "{relative}");
    let decoded = zstd::decode_all(fs::File::open(out.join(relative)).unwrap()).unwrap();
    assert_eq!(decoded, fs::read(server_maps_dir(&server).join("dl_arena.vpk")).unwrap());

    let removed = remove_server_content(&server, &["maps/dl_arena".to_string()], true).unwrap();
    assert_eq!(removed, ["maps/dl_arena"]);
    assert!(!server_maps_dir(&server).join("dl_arena.vpk").exists());
    assert!(remove_server_content(&server, &["maps/dl_midtown".to_string()], true).is_err());
    let config: serde_json::Value =
        serde_json::from_str(&strip_jsonc(&fs::read_to_string(deadworks_config_path(&server)).unwrap())).unwrap();
    assert_eq!(config["serverbrowser"]["extra_maps"], serde_json::json!(["dl_other"]));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn content_checks_refuse_bad_vpks() {
    let root = scratch("content-bad");
    let server = content_server(&root);
    let exe = content_vpk(&root, "evil.vpk", &[("bin/evil.exe", b"MZ")]);
    let badname = content_vpk(&root, "Bad-Name.vpk", &[("scripts/x.vdata_c", b"x")]);
    let split = content_vpk(&root, "pak01_dir.vpk", &[("scripts/x.vdata_c", b"x")]);
    let two = content_vpk(&root, "pack.vpk", &[("maps/dl_a.vmap_c", b"a"), ("maps/dl_b.vmap_c", b"b")]);
    for vpk in [exe, badname, split, two, root.join("missing.vpk")] {
        assert!(install_server_content(&server, std::slice::from_ref(&vpk), true).is_err(), "{}", vpk.display());
    }
    assert!(!server_addons_dir(&server).exists(), "nothing written");
    assert!(install_server_content(&server, &[], false).is_err());
    assert!(install_server_content(&root.join("steamapps/common/x"), &[], true).is_err());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn fastdl_url_is_checked_and_saved() {
    let root = scratch("fastdl");
    let server = content_server(&root);
    assert!(set_fastdl_url(&server, Some("ftp://x"), true).is_err());
    assert!(set_fastdl_url(&server, Some("https://u:p@x.io/a"), true).is_err());
    assert!(set_fastdl_url(&server, Some("https://x.io/a"), false).is_err());
    set_fastdl_url(&server, Some("http://192.168.1.20:27080/"), true).unwrap();
    assert_eq!(fastdl_url(&server).as_deref(), Some("http://192.168.1.20:27080"));
    set_fastdl_url(&server, None, true).unwrap();
    assert_eq!(fastdl_url(&server), None);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn the_download_port_follows_the_game_port() {
    let mut config = ServerConfig::default();
    assert_eq!(config.download_port(), Some(27068));
    config.port = 28000;
    assert_eq!(config.download_port(), Some(28001));
    config.validate().unwrap();
    config.fastdl_port = Some(8080);
    assert_eq!(config.download_port(), Some(8080));
    config.validate().unwrap();
    config.fastdl_port = Some(28000);
    assert!(config.validate().unwrap_err().to_string().contains("differ"));
    config.fastdl_port = Some(80);
    assert!(config.validate().is_err());
    config.fastdl_port = None;
    config.port = 65535;
    assert_eq!(config.download_port(), None);
    assert!(config.validate().is_err());
    // Settings saved before the download port existed still load.
    let old: ServerConfig = serde_json::from_str(r#"{"name":"x","map":"dl_midtown","port":27067}"#).unwrap();
    assert_eq!(old.fastdl_port, None);
    assert_eq!(old.download_port(), Some(27068));
}

/// GET one path from a local server, the status and body.
fn http_get(addr: std::net::SocketAddr, path: &str) -> (u16, Vec<u8>) {
    use std::io::{Read, Write};
    let mut stream = std::net::TcpStream::connect(addr).unwrap();
    stream.set_read_timeout(Some(std::time::Duration::from_secs(10))).unwrap();
    stream.write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes()).unwrap();
    let mut reply = Vec::new();
    stream.read_to_end(&mut reply).unwrap();
    let split = reply.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let status = String::from_utf8_lossy(&reply[9..12]).parse().unwrap();
    (status, reply[split + 4..].to_vec())
}

#[test]
fn starting_with_custom_content_builds_advertises_and_serves_it() {
    let root = scratch("downloads");
    let server = content_server(&root);
    let local: std::net::SocketAddr = "127.0.0.1:0".parse().unwrap();

    // No custom content: nothing is served and nothing is advertised.
    assert!(super::content::start_downloads_on(&server, local).unwrap().is_none());
    assert_eq!(fastdl_url(&server), None);

    let map = content_vpk(&root, "dl_arena.vpk", &[("maps/dl_arena.vmap_c", b"vmap")]);
    install_server_content(&server, &[map], true).unwrap();
    let out = default_fastdl_dir(&server);
    fs::create_dir_all(out.join("maps")).unwrap();
    let stale = out.join("maps").join("dl_old_0123456789abcdef.vpk.zst");
    fs::write(&stale, b"old build").unwrap();
    fs::write(out.join("maps").join("notes.txt"), b"player's file").unwrap();

    let downloads = super::content::start_downloads_on(&server, local).unwrap().expect("serving");
    let port = downloads.port();
    assert_eq!(fastdl_url(&server), Some(format!("http://0.0.0.0:{port}")));
    assert!(!stale.exists(), "an old build of removed content is pruned");
    assert!(out.join("maps").join("notes.txt").exists(), "other files are left alone");

    // What a joiner asks for, at the address it connects to.
    let vpk = fs::read(server_maps_dir(&server).join("dl_arena.vpk")).unwrap();
    let hash = &crate::servers::file_sha256(&server_maps_dir(&server).join("dl_arena.vpk")).unwrap()[..16];
    let entry = crate::servers::ContentEntry {
        kind: crate::servers::ContentKind::Map,
        name: "dl_arena".into(),
        hash: hash.to_string(),
        size: None,
    };
    let base = crate::servers::resolve_fastdl(&fastdl_url(&server).unwrap(), "127.0.0.1".parse().unwrap());
    let url = crate::servers::content_url(&base, &entry, "zst");
    let path = url.strip_prefix(&format!("http://127.0.0.1:{port}")).unwrap();
    let (status, body) = http_get(downloads.local_addr(), path);
    assert_eq!(status, 200);
    assert_eq!(zstd::decode_all(&body[..]).unwrap(), vpk);
    assert_eq!(http_get(downloads.local_addr(), "/maps/notes.txt").0, 404);
    let addr = downloads.local_addr();
    drop(downloads);
    assert!(std::net::TcpStream::connect(addr).is_err(), "dropping it closes the port");

    // A download address the player set themselves is kept, and Sigil serves nothing.
    set_fastdl_url(&server, Some("https://api.deadworks.net/fastdl/u/me"), true).unwrap();
    assert!(super::content::start_downloads_on(&server, local).unwrap().is_none());
    assert_eq!(fastdl_url(&server).as_deref(), Some("https://api.deadworks.net/fastdl/u/me"));

    // Sigil's placeholder is taken out again once the custom content is gone.
    set_fastdl_url(&server, Some(&placeholder_fastdl_url(27068)), true).unwrap();
    remove_server_content(&server, &["maps/dl_arena".to_string()], true).unwrap();
    assert!(super::content::start_downloads_on(&server, local).unwrap().is_none());
    assert_eq!(fastdl_url(&server), None);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn jsonc_strips_comments_and_trailing_commas() {
    let text = "{ // c\n \"a\": \"x // not a comment\", /* b */ \"b\": [1, 2,], \"c\": \"q\\\"/*\",\n}";
    let value: serde_json::Value = serde_json::from_str(&strip_jsonc(text)).unwrap();
    assert_eq!(value["a"], "x // not a comment");
    assert_eq!(value["b"], serde_json::json!([1, 2]));
    assert_eq!(value["c"], "q\"/*");
}
