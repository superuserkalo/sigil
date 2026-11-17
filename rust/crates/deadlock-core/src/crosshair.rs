//! The crosshair model: the convars the game reads, the editor geometry, and
//! the managed block in `autoexec.cfg`.
//!
//! The convar surface is the one the current client registers, recovered from
//! the client and the reference implementation in the vendored Grimoire tree.
//! The old `citadel_crosshair_pip_border` bool is gone: pip and dot outlines are
//! `(border, gap, opacity)` triples sharing an outline colour.
//!
//! Nothing here writes a game file. [`apply_to_autoexec`] returns text, and the
//! caller writes it through the approved autoexec path.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Marker that starts the managed block. Anything between the markers is
/// replaced on apply and removed on clear.
pub const AUTOEXEC_BEGIN: &str = "// dah:crosshair begin";
pub const AUTOEXEC_END: &str = "// dah:crosshair end";

/// Editor starting point. Deliberately different from the game defaults: a
/// sharper, tighter crosshair than a fresh install.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Crosshair {
    pub pip_gap: i32,
    pub pip_gap_static: bool,
    pub pip_height: i32,
    pub pip_width: i32,
    pub pip_opacity: f64,
    pub pip_outline_border: i32,
    pub pip_outline_gap: i32,
    pub pip_outline_opacity: f64,
    pub dot_opacity: f64,
    pub dot_size: i32,
    pub dot_outline_border: i32,
    pub dot_outline_gap: i32,
    pub dot_outline_opacity: f64,
    pub color_r: i32,
    pub color_g: i32,
    pub color_b: i32,
    pub outline_color_r: i32,
    pub outline_color_g: i32,
    pub outline_color_b: i32,
    pub disable_hero_specific_crosshairs: bool,
}

impl Default for Crosshair {
    fn default() -> Self {
        Crosshair {
            pip_gap: 5,
            pip_gap_static: true,
            pip_height: 10,
            pip_width: 2,
            pip_opacity: 1.0,
            pip_outline_border: 1,
            pip_outline_gap: 0,
            pip_outline_opacity: 1.0,
            dot_opacity: 0.0,
            dot_size: 8,
            dot_outline_border: 2,
            dot_outline_gap: 0,
            dot_outline_opacity: 0.0,
            color_r: 0,
            color_g: 255,
            color_b: 0,
            outline_color_r: 0,
            outline_color_g: 0,
            outline_color_b: 0,
            disable_hero_specific_crosshairs: false,
        }
    }
}

impl Crosshair {
    /// The values `client.dll` registers on a fresh install.
    pub fn game_defaults() -> Self {
        Crosshair {
            pip_gap: 4,
            pip_gap_static: false,
            pip_height: 16,
            pip_width: 2,
            pip_opacity: 0.5,
            pip_outline_border: 1,
            pip_outline_gap: 1,
            pip_outline_opacity: 0.7,
            dot_opacity: 0.7,
            dot_size: 4,
            dot_outline_border: 2,
            dot_outline_gap: 2,
            dot_outline_opacity: 0.7,
            color_r: 255,
            color_g: 255,
            color_b: 255,
            outline_color_r: 0,
            outline_color_g: 0,
            outline_color_b: 0,
            disable_hero_specific_crosshairs: false,
        }
    }

    /// Clamp every field to the range the game accepts. Negative size or gap
    /// values make the HUD element collapse, so they are refused, not clamped
    /// silently in the command text.
    pub fn validate(&self) -> Result<()> {
        let ranges = [
            ("pip_gap", self.pip_gap, 0, 64),
            ("pip_height", self.pip_height, 0, 64),
            ("pip_width", self.pip_width, 0, 64),
            ("pip_outline_border", self.pip_outline_border, 0, 16),
            ("pip_outline_gap", self.pip_outline_gap, 0, 16),
            ("dot_size", self.dot_size, 0, 64),
            ("dot_outline_border", self.dot_outline_border, 0, 16),
            ("dot_outline_gap", self.dot_outline_gap, 0, 16),
        ];
        for (name, value, low, high) in ranges {
            if !(low..=high).contains(&value) {
                return Err(Error::msg(format!(
                    "Crosshair {name} is {value}; the game accepts {low} to {high}."
                )));
            }
        }
        for (name, value) in [
            ("pip_opacity", self.pip_opacity),
            ("pip_outline_opacity", self.pip_outline_opacity),
            ("dot_opacity", self.dot_opacity),
            ("dot_outline_opacity", self.dot_outline_opacity),
        ] {
            if !(0.0..=1.0).contains(&value) {
                return Err(Error::msg(format!(
                    "Crosshair {name} is {value}; opacity is 0 to 1."
                )));
            }
        }
        for (name, value) in [
            ("color_r", self.color_r),
            ("color_g", self.color_g),
            ("color_b", self.color_b),
            ("outline_color_r", self.outline_color_r),
            ("outline_color_g", self.outline_color_g),
            ("outline_color_b", self.outline_color_b),
        ] {
            if !(0..=255).contains(&value) {
                return Err(Error::msg(format!(
                    "Crosshair {name} is {value}; colour channels are 0 to 255."
                )));
            }
        }
        Ok(())
    }

    /// The full convar set. Emitting all of it is deliberate: a partial write
    /// leaves the rest of a previous crosshair in place.
    pub fn commands(&self) -> Vec<String> {
        vec![
            format!("citadel_crosshair_pip_gap {}", self.pip_gap),
            format!("citadel_crosshair_pip_gap_static {}", self.pip_gap_static),
            format!("citadel_crosshair_pip_height {}", self.pip_height),
            format!("citadel_crosshair_pip_width {}", self.pip_width),
            format!("citadel_crosshair_pip_opacity {}", opacity(self.pip_opacity)),
            format!(
                "citadel_crosshair_pip_outline_border {}",
                self.pip_outline_border
            ),
            format!("citadel_crosshair_pip_outline_gap {}", self.pip_outline_gap),
            format!(
                "citadel_crosshair_pip_outline_opacity {}",
                opacity(self.pip_outline_opacity)
            ),
            format!("citadel_crosshair_dot_size {}", self.dot_size),
            format!("citadel_crosshair_dot_opacity {}", opacity(self.dot_opacity)),
            format!(
                "citadel_crosshair_dot_outline_border {}",
                self.dot_outline_border
            ),
            format!("citadel_crosshair_dot_outline_gap {}", self.dot_outline_gap),
            format!(
                "citadel_crosshair_dot_outline_opacity {}",
                opacity(self.dot_outline_opacity)
            ),
            format!("citadel_crosshair_color_r {}", self.color_r),
            format!("citadel_crosshair_color_g {}", self.color_g),
            format!("citadel_crosshair_color_b {}", self.color_b),
            format!("citadel_crosshair_outline_color_r {}", self.outline_color_r),
            format!("citadel_crosshair_outline_color_g {}", self.outline_color_g),
            format!("citadel_crosshair_outline_color_b {}", self.outline_color_b),
            format!(
                "citadel_crosshair_disable_hero_specific_crosshairs {}",
                self.disable_hero_specific_crosshairs
            ),
        ]
    }

    pub fn command_text(&self) -> String {
        self.commands().join("\n")
    }

    /// Every command on one line, joined with `; `. The in-game console runs only the
    /// first line of a paste, so this is the form a player pastes there.
    pub fn console_line(&self) -> String {
        self.commands().join("; ")
    }

    /// The settings as the query `api.deadlock-api.com` takes to encode a share code:
    /// each convar without its `citadel_crosshair_` prefix. The hero specific toggle is
    /// not part of a code; `themed=false` keeps the code's own look.
    pub fn share_code_query(&self) -> Vec<(&'static str, String)> {
        let flag = |value: bool| value.to_string();
        // Plain numbers, as Grimoire sends them: `1` and `0.7`, not `1.00`.
        let float = |value: f64| ((value * 100.0).round() / 100.0).to_string();
        vec![
            ("themed", "false".to_string()),
            ("pip_gap", self.pip_gap.to_string()),
            ("pip_gap_static", flag(self.pip_gap_static)),
            ("pip_height", self.pip_height.to_string()),
            ("pip_width", self.pip_width.to_string()),
            ("pip_opacity", float(self.pip_opacity)),
            ("pip_outline_border", self.pip_outline_border.to_string()),
            ("pip_outline_gap", self.pip_outline_gap.to_string()),
            ("pip_outline_opacity", float(self.pip_outline_opacity)),
            ("dot_size", self.dot_size.to_string()),
            ("dot_opacity", float(self.dot_opacity)),
            ("dot_outline_border", self.dot_outline_border.to_string()),
            ("dot_outline_gap", self.dot_outline_gap.to_string()),
            ("dot_outline_opacity", float(self.dot_outline_opacity)),
            ("color_r", self.color_r.to_string()),
            ("color_g", self.color_g.to_string()),
            ("color_b", self.color_b.to_string()),
            ("outline_color_r", self.outline_color_r.to_string()),
            ("outline_color_g", self.outline_color_g.to_string()),
            ("outline_color_b", self.outline_color_b.to_string()),
        ]
    }

    /// Rasterize the crosshair the way the HUD does, in 1080p layout units
    /// scaled once to device pixels. `scale` is display height / 1080.
    pub fn layout(&self, scale: f32) -> Layout {
        let pip_width = self.pip_width.max(0) as f32;
        let pip_height = self.pip_height.max(0) as f32;
        let gap = self.pip_gap.max(0) as f32;
        let outline_gap = self.pip_outline_gap.max(0) as f32;
        let outline_border = self.pip_outline_border.max(0) as f32;
        let dot = self.dot_size.max(0) as f32;
        let dot_gap = self.dot_outline_gap.max(0) as f32;
        let dot_border = self.dot_outline_border.max(0) as f32;

        // `cppArcDefaultOffset` in element_gun.vcss.
        let distance = 4.0 + gap;
        let dev = |value: f32| (value * scale).round();
        let edge = |centre: f32, size: f32| (centre * scale - size / 2.0).round();

        let specs: [(f32, f32, f32, f32); 4] = if pip_width > 0.0 && pip_height > 0.0 {
            [
                (0.0, -distance, pip_width, pip_height),
                (0.0, distance, pip_width, pip_height),
                (-distance, 0.0, pip_height, pip_width),
                (distance, 0.0, pip_height, pip_width),
            ]
        } else {
            [(0.0, 0.0, 0.0, 0.0); 4]
        };

        let rect = |cx: f32, cy: f32, w: f32, h: f32, grow: f32| {
            let width = dev(w + grow);
            let height = dev(h + grow);
            Rect {
                x: edge(cx, width),
                y: edge(cy, height),
                w: width,
                h: height,
            }
        };

        let pips: Vec<Rect> = specs
            .iter()
            .map(|(cx, cy, w, h)| rect(*cx, *cy, *w, *h, 0.0))
            .collect();
        let pip_outlines: Vec<Rect> = specs
            .iter()
            .map(|(cx, cy, w, h)| rect(*cx, *cy, *w, *h, outline_gap + 2.0 * outline_border))
            .collect();
        let dot_rect = rect(0.0, 0.0, dot, dot, 0.0);
        let dot_outline = rect(0.0, 0.0, dot, dot, dot_gap + 2.0 * dot_border);

        Layout {
            pips,
            pip_outlines,
            pip_outline_border: dev(outline_border),
            dot: dot_rect,
            dot_outline,
            dot_outline_border: dev(dot_border),
            // The preview canvas needs a radius that always contains the art.
            reach: (distance + pip_height / 2.0 + outline_gap / 2.0 + outline_border)
                .max((dot + dot_gap) / 2.0 + dot_border)
                + 4.0,
        }
    }
}

impl fmt::Display for Crosshair {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "pip {}x{} gap {} · dot {} · rgb({},{},{})",
            self.pip_width,
            self.pip_height,
            self.pip_gap,
            self.dot_size,
            self.color_r,
            self.color_g,
            self.color_b
        )
    }
}

fn opacity(value: f64) -> String {
    format!("{value:.2}")
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub pips: Vec<Rect>,
    pub pip_outlines: Vec<Rect>,
    pub pip_outline_border: f32,
    pub dot: Rect,
    pub dot_outline: Rect,
    pub dot_outline_border: f32,
    pub reach: f32,
}

/// Where the game's `DL.` share codes are encoded. The host is on the network allowlist.
pub const SHARE_CODE_URL: &str = "https://api.deadlock-api.com/v1/crosshair/settings/code";

/// Where the game's `DL.` share codes are decoded back into settings. Same host.
pub const SHARE_DECODE_URL: &str = "https://api.deadlock-api.com/v1/crosshair/code/settings";

/// Encode `crosshair` as the game's `DL.` share code through deadlock-api. Only the
/// crosshair values are sent, on an explicit player action. Blocking: call it from a worker.
pub fn fetch_share_code(crosshair: &Crosshair) -> Result<String> {
    crosshair.validate()?;
    let body = share_api_get(SHARE_CODE_URL, crosshair.share_code_query())?;
    parse_share_code(&body)
}

/// Decode a `DL.` share code into settings through deadlock-api. Only the code is sent, and
/// only when it has the shape of a code. The hero specific toggle is not part of a code, so
/// it keeps its value from `current`. Blocking: call it from a worker.
pub fn decode_share_code(code: &str, current: &Crosshair) -> Result<Crosshair> {
    let code = code.trim();
    if !is_share_code(code) {
        return Err(Error::msg("That is not a crosshair share code."));
    }
    let body = share_api_get(SHARE_DECODE_URL, vec![("code", code.to_string())])?;
    crosshair_from_share_reply(&body, current)
}

fn share_api_get(url: &str, query: Vec<(&'static str, String)>) -> Result<String> {
    use std::time::Duration;
    let agent = crate::net::builder()
        .middleware(crate::net::gate)
        .timeout_global(Some(Duration::from_secs(20)))
        .http_status_as_error(false)
        .build()
        .new_agent();
    let mut request = agent
        .get(url)
        .header("User-Agent", "dah/0.0.0")
        .header("Accept", "application/json");
    for (name, value) in query {
        request = request.query(name, value);
    }
    let mut response = request
        .call()
        .map_err(|error| Error::Network(format!("The share code request failed: {error}")))?;
    let status = response.status().as_u16();
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|error| Error::Network(format!("The share code reply was unreadable: {error}")))?;
    if !(200..300).contains(&status) {
        return Err(Error::Network(format!(
            "The share code service returned HTTP {status}."
        )));
    }
    Ok(body)
}

/// Text with the shape of a game share code. Anything else is never sent anywhere.
/// The same test as Grimoire's `isCrosshairShareCode`.
pub fn is_share_code(text: &str) -> bool {
    let Some(rest) = text.strip_prefix("DL.") else {
        return false;
    };
    (8..=4096).contains(&rest.len())
        && rest
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"+/=_.-".contains(&b))
}

/// Settings from a decoded share code reply: the share code field names, each a number or a
/// bool. Fields the reply leaves out keep their value from `current`. The result is checked
/// against the ranges the game accepts.
pub fn crosshair_from_share_reply(body: &str, current: &Crosshair) -> Result<Crosshair> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|error| Error::msg(format!("The share code reply was not JSON: {error}")))?;
    let object = value
        .as_object()
        .ok_or_else(|| Error::msg("The share code reply had no settings in it."))?;
    let mut crosshair = current.clone();
    let mut found = 0;
    for (name, _) in current.share_code_query() {
        let Some(field) = object.get(name) else {
            continue;
        };
        let text = match field {
            serde_json::Value::Bool(flag) => flag.to_string(),
            serde_json::Value::Number(number) => number.to_string(),
            serde_json::Value::String(text) => text.clone(),
            _ => continue,
        };
        if set_convar(&mut crosshair, name, &text)? {
            found += 1;
        }
    }
    if found == 0 {
        return Err(Error::msg("The share code reply had no settings in it."));
    }
    crosshair.validate()?;
    Ok(crosshair)
}

// --- import --------------------------------------------------------------------------------

/// The longest paste the importer reads. A full console line is under 1 KB.
pub const IMPORT_MAX: usize = 16 * 1024;

/// What a pasted crosshair turned out to be.
#[derive(Debug, Clone, PartialEq)]
pub enum Import {
    /// A `DL.` share code. Decoding it needs [`decode_share_code`].
    Code(String),
    /// Console commands or config lines, already parsed and checked.
    Settings(Crosshair),
}

/// Read a pasted crosshair: a `DL.` share code, the one line console form
/// (`citadel_crosshair_pip_gap 5; citadel_crosshair_color_r 0; ...`), an `autoexec.cfg`
/// block, or `machine_convars.vcfg` lines. Commands that are not crosshair convars are
/// skipped; a crosshair convar with a bad value, or settings out of the game's range, are
/// refused. Convars the paste leaves out keep their value from `current`, as in game.
pub fn parse_import(text: &str, current: &Crosshair) -> Result<Import> {
    let text = text.trim();
    if text.is_empty() {
        return Err(Error::msg("Paste a share code or crosshair commands."));
    }
    if text.len() > IMPORT_MAX {
        return Err(Error::msg("That paste is too long to be a crosshair."));
    }
    if text.starts_with("DL.") {
        return if is_share_code(text) {
            Ok(Import::Code(text.to_string()))
        } else {
            Err(Error::msg("That share code is incomplete or has stray characters."))
        };
    }
    parse_console(text, current).map(Import::Settings)
}

/// Console commands or config lines to settings. See [`parse_import`].
pub fn parse_console(text: &str, current: &Crosshair) -> Result<Crosshair> {
    let mut crosshair = current.clone();
    let mut found = 0;
    for statement in text.split(['\n', ';']) {
        let statement = match statement.find("//") {
            Some(at) => &statement[..at],
            None => statement,
        };
        let mut tokens = statement
            .split_whitespace()
            .map(|token| token.trim_matches('"'))
            .filter(|token| !token.is_empty());
        let Some(key) = tokens.next() else {
            continue;
        };
        let Some(name) = key.strip_prefix("citadel_crosshair_") else {
            continue;
        };
        let Some(value) = tokens.next() else {
            return Err(Error::msg(format!("{key} has no value.")));
        };
        if set_convar(&mut crosshair, name, value)? {
            found += 1;
        }
    }
    if found == 0 {
        return Err(Error::msg(
            "No crosshair settings found. Paste a DL share code or citadel_crosshair commands.",
        ));
    }
    crosshair.validate()?;
    Ok(crosshair)
}

/// Set one convar by its name without the `citadel_crosshair_` prefix. `Ok(false)` for a
/// name this model does not know; an error for a known name with a value of the wrong kind.
fn set_convar(c: &mut Crosshair, name: &str, value: &str) -> Result<bool> {
    let full = format!("citadel_crosshair_{name}");
    let number = || -> Result<f64> {
        value
            .parse::<f64>()
            .ok()
            .filter(|number| number.is_finite())
            .ok_or_else(|| Error::msg(format!("{full} needs a number, not \"{value}\".")))
    };
    let int = || -> Result<i32> {
        let number = number()?;
        if number.fract() != 0.0 || number.abs() > 100_000.0 {
            return Err(Error::msg(format!("{full} needs a whole number, not \"{value}\".")));
        }
        Ok(number as i32)
    };
    let fraction = || -> Result<f64> { Ok((number()? * 100.0).round() / 100.0) };
    let flag = || -> Result<bool> {
        match value.to_ascii_lowercase().as_str() {
            "true" | "1" => Ok(true),
            "false" | "0" => Ok(false),
            _ => Err(Error::msg(format!("{full} needs true or false, not \"{value}\"."))),
        }
    };
    match name {
        "pip_gap" => c.pip_gap = int()?,
        "pip_gap_static" => c.pip_gap_static = flag()?,
        "pip_height" => c.pip_height = int()?,
        "pip_width" => c.pip_width = int()?,
        "pip_opacity" => c.pip_opacity = fraction()?,
        "pip_outline_border" => c.pip_outline_border = int()?,
        "pip_outline_gap" => c.pip_outline_gap = int()?,
        "pip_outline_opacity" => c.pip_outline_opacity = fraction()?,
        "dot_size" => c.dot_size = int()?,
        "dot_opacity" => c.dot_opacity = fraction()?,
        "dot_outline_border" => c.dot_outline_border = int()?,
        "dot_outline_gap" => c.dot_outline_gap = int()?,
        "dot_outline_opacity" => c.dot_outline_opacity = fraction()?,
        "color_r" => c.color_r = int()?,
        "color_g" => c.color_g = int()?,
        "color_b" => c.color_b = int()?,
        "outline_color_r" => c.outline_color_r = int()?,
        "outline_color_g" => c.outline_color_g = int()?,
        "outline_color_b" => c.outline_color_b = int()?,
        "disable_hero_specific_crosshairs" => c.disable_hero_specific_crosshairs = flag()?,
        // The pre outline bool, as older configs and Grimoire's legacy presets carry it.
        "pip_border" => c.pip_outline_border = if flag()? { 1 } else { 0 },
        _ => return Ok(false),
    }
    Ok(true)
}

// --- presets -------------------------------------------------------------------------------

/// The saved presets file in the app data directory.
pub const PRESETS_FILE: &str = "crosshair-presets.json";
pub const PRESET_NAME_MAX: usize = 40;
pub const PRESETS_MAX: usize = 200;
/// A presets file larger than this is refused rather than parsed.
const PRESETS_FILE_MAX: u64 = 2 * 1024 * 1024;

/// A crosshair the player saved. The shape follows Grimoire's `crosshair-presets.json`
/// (`id`, `name`, camelCase `settings`, `createdAt`), so a Grimoire file reads as is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Preset {
    pub id: String,
    pub name: String,
    pub settings: Crosshair,
    #[serde(default)]
    pub created_at: String,
}

#[derive(Serialize, Deserialize)]
struct PresetFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    presets: Vec<serde_json::Value>,
}

/// A preset that ships with Sigil.
#[derive(Debug, Clone, PartialEq)]
pub struct BuiltinPreset {
    pub id: &'static str,
    pub name: &'static str,
    pub settings: Crosshair,
}

/// The presets that ship with Sigil. "Default" is the editor starting point Grimoire uses
/// (`CROSSHAIR_DEFAULTS`) and "Game default" is what the client registers on a fresh install
/// (`CROSSHAIR_GAME_DEFAULTS`); the rest are Sigil's own. Neither Grimoire nor Deadlock Mod
/// Manager ships other named presets.
pub fn builtin_presets() -> Vec<BuiltinPreset> {
    let base = Crosshair::default();
    let white = |c: Crosshair| Crosshair {
        color_r: 255,
        color_g: 255,
        color_b: 255,
        ..c
    };
    vec![
        BuiltinPreset {
            id: "default",
            name: "Default",
            settings: base.clone(),
        },
        BuiltinPreset {
            id: "game",
            name: "Game default",
            settings: Crosshair::game_defaults(),
        },
        BuiltinPreset {
            id: "dot",
            name: "Dot",
            settings: white(Crosshair {
                pip_opacity: 0.0,
                pip_outline_opacity: 0.0,
                dot_size: 4,
                dot_opacity: 1.0,
                dot_outline_border: 1,
                dot_outline_gap: 0,
                dot_outline_opacity: 1.0,
                ..base.clone()
            }),
        },
        BuiltinPreset {
            id: "small-cross",
            name: "Small cross",
            settings: white(Crosshair {
                pip_gap: 2,
                pip_height: 6,
                pip_width: 2,
                ..base.clone()
            }),
        },
        BuiltinPreset {
            id: "cross-dot",
            name: "Cross and dot",
            settings: Crosshair {
                pip_gap: 4,
                pip_height: 8,
                dot_size: 2,
                dot_opacity: 1.0,
                dot_outline_border: 1,
                dot_outline_opacity: 1.0,
                color_r: 0,
                color_g: 255,
                color_b: 255,
                ..base.clone()
            },
        },
        BuiltinPreset {
            id: "wide",
            name: "Wide gap",
            settings: Crosshair {
                pip_gap: 14,
                pip_height: 10,
                color_r: 255,
                color_g: 190,
                color_b: 0,
                ..base.clone()
            },
        },
        BuiltinPreset {
            id: "thin",
            name: "Thin",
            settings: Crosshair {
                pip_gap: 4,
                pip_height: 12,
                pip_width: 1,
                pip_outline_border: 0,
                pip_outline_opacity: 0.0,
                color_r: 255,
                color_g: 60,
                color_b: 220,
                ..base.clone()
            },
        },
        BuiltinPreset {
            id: "bold",
            name: "Bold",
            settings: Crosshair {
                pip_gap: 6,
                pip_height: 12,
                pip_width: 4,
                color_r: 255,
                color_g: 60,
                color_b: 60,
                ..base
            },
        },
    ]
}

/// `name` tidied for a preset: control characters dropped, runs of space collapsed, at most
/// [`PRESET_NAME_MAX`] characters. Empty is refused.
pub fn clean_preset_name(name: &str) -> Result<String> {
    let cleaned = name
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if cleaned.is_empty() {
        return Err(Error::msg("Give the preset a name."));
    }
    if cleaned.chars().count() > PRESET_NAME_MAX {
        return Err(Error::msg(format!(
            "Keep the name under {} characters.",
            PRESET_NAME_MAX + 1
        )));
    }
    Ok(cleaned)
}

fn name_taken(presets: &[Preset], name: &str, except: Option<&str>) -> bool {
    presets
        .iter()
        .filter(|preset| Some(preset.id.as_str()) != except)
        .any(|preset| preset.name.eq_ignore_ascii_case(name))
}

/// Add `settings` as a new preset called `name`. Returns the new preset's id.
pub fn add_preset(presets: &mut Vec<Preset>, name: &str, settings: &Crosshair) -> Result<String> {
    let name = clean_preset_name(name)?;
    settings.validate()?;
    if name_taken(presets, &name, None) {
        return Err(Error::msg(format!("A preset named {name} already exists.")));
    }
    if presets.len() >= PRESETS_MAX {
        return Err(Error::msg(format!(
            "There are already {PRESETS_MAX} presets. Delete one first."
        )));
    }
    let stamp = chrono::Utc::now().timestamp_millis();
    let mut n = presets.len();
    let id = loop {
        let id = format!("preset_{stamp}_{n}");
        if !presets.iter().any(|preset| preset.id == id) {
            break id;
        }
        n += 1;
    };
    presets.push(Preset {
        id: id.clone(),
        name,
        settings: settings.clone(),
        created_at: crate::util::now_iso(),
    });
    Ok(id)
}

/// Rename the preset `id`. A missing id is an error, as is a name another preset uses.
pub fn rename_preset(presets: &mut [Preset], id: &str, name: &str) -> Result<()> {
    let name = clean_preset_name(name)?;
    if name_taken(presets, &name, Some(id)) {
        return Err(Error::msg(format!("A preset named {name} already exists.")));
    }
    let preset = presets
        .iter_mut()
        .find(|preset| preset.id == id)
        .ok_or_else(|| Error::msg("That preset no longer exists."))?;
    preset.name = name;
    Ok(())
}

/// Remove the preset `id`. `false` when there was none.
pub fn remove_preset(presets: &mut Vec<Preset>, id: &str) -> bool {
    let before = presets.len();
    presets.retain(|preset| preset.id != id);
    presets.len() != before
}

/// Parse a presets file: Sigil's `{ "version", "presets" }` or a bare array. Entries that
/// do not parse, have no usable name, repeat an id, or hold settings the game would refuse
/// are dropped, so one bad entry never loses the rest.
pub fn parse_presets(text: &str) -> Result<Vec<Preset>> {
    let value: serde_json::Value = serde_json::from_str(text)
        .map_err(|error| Error::msg(format!("The presets file is not valid JSON: {error}")))?;
    let entries = match value {
        serde_json::Value::Array(entries) => entries,
        other => serde_json::from_value::<PresetFile>(other)
            .map_err(|error| Error::msg(format!("The presets file has an unknown shape: {error}")))?
            .presets,
    };
    let mut presets: Vec<Preset> = Vec::new();
    for entry in entries {
        let Ok(mut preset) = serde_json::from_value::<Preset>(entry) else {
            continue;
        };
        let Ok(name) = clean_preset_name(&preset.name) else {
            continue;
        };
        preset.name = name;
        if preset.id.trim().is_empty()
            || presets.iter().any(|kept| kept.id == preset.id)
            || preset.settings.validate().is_err()
            || presets.len() >= PRESETS_MAX
        {
            continue;
        }
        presets.push(preset);
    }
    Ok(presets)
}

/// Read the saved presets. A missing file is an empty list.
pub fn load_presets(path: &std::path::Path) -> Result<Vec<Preset>> {
    let meta = match std::fs::metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    if meta.len() > PRESETS_FILE_MAX {
        return Err(Error::msg("The presets file is too large to read."));
    }
    parse_presets(&std::fs::read_to_string(path)?)
}

/// Write the saved presets through a temporary file, so a crash never leaves half a file.
pub fn save_presets(path: &std::path::Path, presets: &[Preset]) -> Result<()> {
    let file = serde_json::json!({ "version": 1, "presets": presets });
    let mut text = serde_json::to_string_pretty(&file)?;
    text.push('\n');
    crate::util::write_atomic(path, text.as_bytes())
}

/// The `code` field of a share code reply, refused unless it looks like a game code.
pub fn parse_share_code(body: &str) -> Result<String> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|error| Error::msg(format!("The share code reply was not JSON: {error}")))?;
    let code = value
        .get("code")
        .and_then(|code| code.as_str())
        .unwrap_or_default()
        .trim();
    let valid = code.len() > 8
        && code.starts_with("DL.")
        && code
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"+/=_.-".contains(&b));
    if valid {
        Ok(code.to_string())
    } else {
        Err(Error::msg("The share code reply had no code in it."))
    }
}

/// Read the crosshair the player set in game from `machine_convars.vcfg`.
///
/// Returns `None` when the text has no crosshair convar at all, which is
/// different from "the file uses the game defaults".
pub fn parse_machine_convars(text: &str) -> Option<Crosshair> {
    let mut crosshair = Crosshair::default();
    let mut found = false;

    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix('"') else {
            continue;
        };
        let Some((key, value)) = rest.split_once('"') else {
            continue;
        };
        let value = value.trim().trim_start_matches('"').trim_end_matches('"');
        let value = value.trim_end_matches('\\').trim();
        let Some(field) = key.strip_prefix("citadel_crosshair_") else {
            continue;
        };
        found = true;
        match field {
            "pip_gap" => crosshair.pip_gap = parse_int(value, crosshair.pip_gap),
            "pip_gap_static" => crosshair.pip_gap_static = parse_bool(value),
            "pip_height" => crosshair.pip_height = parse_int(value, crosshair.pip_height),
            "pip_width" => crosshair.pip_width = parse_int(value, crosshair.pip_width),
            "pip_opacity" => crosshair.pip_opacity = parse_float(value, crosshair.pip_opacity),
            "pip_outline_border" => {
                crosshair.pip_outline_border = parse_int(value, crosshair.pip_outline_border)
            }
            "pip_outline_gap" => {
                crosshair.pip_outline_gap = parse_int(value, crosshair.pip_outline_gap)
            }
            "pip_outline_opacity" => {
                crosshair.pip_outline_opacity = parse_float(value, crosshair.pip_outline_opacity)
            }
            "dot_size" => crosshair.dot_size = parse_int(value, crosshair.dot_size),
            "dot_opacity" => crosshair.dot_opacity = parse_float(value, crosshair.dot_opacity),
            "dot_outline_border" => {
                crosshair.dot_outline_border = parse_int(value, crosshair.dot_outline_border)
            }
            "dot_outline_gap" => {
                crosshair.dot_outline_gap = parse_int(value, crosshair.dot_outline_gap)
            }
            "dot_outline_opacity" => {
                crosshair.dot_outline_opacity = parse_float(value, crosshair.dot_outline_opacity)
            }
            "color_r" => crosshair.color_r = parse_int(value, crosshair.color_r),
            "color_g" => crosshair.color_g = parse_int(value, crosshair.color_g),
            "color_b" => crosshair.color_b = parse_int(value, crosshair.color_b),
            "outline_color_r" => {
                crosshair.outline_color_r = parse_int(value, crosshair.outline_color_r)
            }
            "outline_color_g" => {
                crosshair.outline_color_g = parse_int(value, crosshair.outline_color_g)
            }
            "outline_color_b" => {
                crosshair.outline_color_b = parse_int(value, crosshair.outline_color_b)
            }
            "disable_hero_specific_crosshairs" => {
                crosshair.disable_hero_specific_crosshairs = parse_bool(value)
            }
            _ => {}
        }
    }

    if found { Some(crosshair) } else { None }
}

fn parse_int(value: &str, fallback: i32) -> i32 {
    value
        .parse::<f64>()
        .ok()
        .map(|value| value.trunc() as i32)
        .unwrap_or(fallback)
}

fn parse_float(value: &str, fallback: f64) -> f64 {
    value.parse::<f64>().unwrap_or(fallback)
}

fn parse_bool(value: &str) -> bool {
    value == "true" || value == "1"
}

/// Replace the managed block in an autoexec file, keeping every other line.
///
/// The block is appended when it is absent. A file with a begin marker and no
/// end marker is refused rather than guessed at.
pub fn apply_to_autoexec(existing: &str, crosshair: &Crosshair) -> Result<String> {
    crosshair.validate()?;
    let block = format!(
        "{AUTOEXEC_BEGIN}\n{}\n{AUTOEXEC_END}",
        crosshair.command_text()
    );
    let (before, after) = split_block(existing)?;
    let mut output = String::new();
    let head = before.trim_end_matches(['\r', '\n']);
    if !head.is_empty() {
        output.push_str(head);
        output.push('\n');
    }
    output.push_str(&block);
    output.push('\n');
    let tail = after.trim_start_matches(['\r', '\n']);
    if !tail.is_empty() {
        output.push_str(tail);
        output.push('\n');
    }
    Ok(output)
}

/// Remove the managed block, keeping every other line.
pub fn clear_from_autoexec(existing: &str) -> Result<String> {
    let (before, after) = split_block(existing)?;
    let mut output = String::new();
    let head = before.trim_end_matches(['\r', '\n']);
    if !head.is_empty() {
        output.push_str(head);
        output.push('\n');
    }
    let tail = after.trim_start_matches(['\r', '\n']);
    if !tail.is_empty() {
        output.push_str(tail);
    }
    Ok(output)
}

fn split_block(existing: &str) -> Result<(&str, &str)> {
    let Some(begin) = existing.find(AUTOEXEC_BEGIN) else {
        return Ok((existing, ""));
    };
    let Some(relative_end) = existing[begin..].find(AUTOEXEC_END) else {
        return Err(Error::msg(format!(
            "autoexec.cfg has {AUTOEXEC_BEGIN} without {AUTOEXEC_END}. Fix the block by hand; nothing was merged."
        )));
    };
    let end = begin + relative_end + AUTOEXEC_END.len();
    Ok((&existing[..begin], &existing[end..]))
}

/// GAME WRITE. Take the crosshair block out of the install's `autoexec.cfg`, keeping every
/// other line. Called only from the Crosshair page's "Remove from autoexec" confirm. Returns
/// `None` when there is no file or no block, so nothing is written. Otherwise
/// `autoexec::write_autoexec` copies the old file aside and writes atomically.
pub fn remove_from_install(
    game_root: &std::path::Path,
) -> Result<Option<crate::autoexec::AutoexecWriteResult>> {
    let Some(existing) = crate::autoexec::read_autoexec(game_root)? else {
        return Ok(None);
    };
    if !existing.contains(AUTOEXEC_BEGIN) {
        return Ok(None);
    }
    let cleared = clear_from_autoexec(&existing)?;
    crate::autoexec::write_autoexec(game_root, &cleared, true).map(Some)
}

/// Whether the install's `autoexec.cfg` holds a crosshair block. Read only.
pub fn applied_in_install(game_root: &std::path::Path) -> bool {
    crate::autoexec::read_autoexec(game_root)
        .ok()
        .flatten()
        .is_some_and(|text| text.contains(AUTOEXEC_BEGIN))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_commands_cover_every_convar() {
        let commands = Crosshair::default().commands();
        assert_eq!(commands.len(), 20);
        assert!(commands[0].starts_with("citadel_crosshair_pip_gap"));
        assert!(commands[8].contains("dot_size"));
    }

    #[test]
    fn opacity_is_written_with_two_decimals() {
        let mut crosshair = Crosshair::default();
        crosshair.dot_opacity = 0.7;
        crosshair.dot_outline_opacity = 1.0;
        let text = crosshair.command_text();
        assert!(text.contains("citadel_crosshair_dot_opacity 0.70"));
        assert!(text.contains("citadel_crosshair_dot_outline_opacity 1.00"));
    }

    #[test]
    fn rejects_a_value_the_game_cannot_use() {
        let mut crosshair = Crosshair::default();
        crosshair.pip_gap = -3;
        assert!(crosshair.validate().is_err());
        crosshair.pip_gap = 5;
        crosshair.pip_opacity = 1.5;
        assert!(crosshair.validate().is_err());
    }

    #[test]
    fn parses_a_machine_convars_file() {
        let text = r#"
"citadel_crosshair_pip_gap"		"9"
"citadel_crosshair_pip_height"		"12"
"citadel_crosshair_pip_width"		"3"
"citadel_crosshair_pip_opacity"		"0.85"
"citadel_crosshair_dot_opacity"		"1.0"
"citadel_crosshair_color_r"		"255"
"citadel_crosshair_color_g"		"0"
"citadel_crosshair_disable_hero_specific_crosshairs"		"1"
"unrelated_setting"		"5"
"#;
        let crosshair = parse_machine_convars(text).expect("crosshair data");
        assert_eq!(crosshair.pip_gap, 9);
        assert_eq!(crosshair.pip_width, 3);
        assert_eq!(crosshair.dot_opacity, 1.0);
        assert_eq!(crosshair.color_r, 255);
        assert!(crosshair.disable_hero_specific_crosshairs);
    }

    #[test]
    fn a_file_without_crosshair_data_is_none() {
        assert!(parse_machine_convars("\"setting\" \"1\"\n").is_none());
    }

    #[test]
    fn apply_and_clear_keep_unrelated_lines() {
        let existing = "bind \"F1\" \"disconnect\"\nrate 128000\n";
        let applied = apply_to_autoexec(existing, &Crosshair::default()).expect("apply");
        assert!(applied.contains("bind \"F1\" \"disconnect\""));
        assert!(applied.contains(AUTOEXEC_BEGIN));
        assert!(applied.contains(AUTOEXEC_END));

        let again = apply_to_autoexec(&applied, &Crosshair::default()).expect("re-apply");
        assert_eq!(again.matches(AUTOEXEC_BEGIN).count(), 1);

        let cleared = clear_from_autoexec(&again).expect("clear");
        assert!(!cleared.contains(AUTOEXEC_BEGIN));
        assert!(cleared.contains("bind \"F1\" \"disconnect\""));
    }

    #[test]
    fn an_unbalanced_block_is_refused() {
        let text = format!("{AUTOEXEC_BEGIN}\ncitadel_crosshair_pip_gap 5\n");
        assert!(apply_to_autoexec(&text, &Crosshair::default()).is_err());
    }

    #[test]
    fn layout_is_symmetric_around_the_centre() {
        let layout = Crosshair::default().layout(1.0);
        assert_eq!(layout.pips.len(), 4);
        let top = layout.pips[0];
        let bottom = layout.pips[1];
        assert_eq!(top.w, bottom.w);
        assert!((top.x + top.w / 2.0).abs() < 0.001);
        assert!(((top.y + top.h / 2.0) + (bottom.y + bottom.h / 2.0)).abs() < 0.001);
    }

    #[test]
    fn the_console_line_is_one_line() {
        let line = Crosshair::default().console_line();
        assert!(!line.contains('\n'));
        assert_eq!(line.matches("; ").count(), 19);
    }

    #[test]
    fn the_share_code_query_names_every_convar_but_the_hero_toggle() {
        let query = Crosshair::default().share_code_query();
        assert_eq!(query.len(), 20);
        assert_eq!(query[0], ("themed", "false".to_string()));
        assert!(query.iter().all(|(name, _)| !name.starts_with("citadel_")));
        assert!(!query.iter().any(|(name, _)| name.contains("hero")));
        assert!(query.contains(&("pip_opacity", "1".to_string())));
    }

    #[test]
    fn a_share_code_reply_is_checked() {
        assert_eq!(
            parse_share_code(r#"{"code":"DL.AQAiFvB_AQK4NAL5"}"#).unwrap(),
            "DL.AQAiFvB_AQK4NAL5"
        );
        assert!(parse_share_code(r#"{"code":"<script>"}"#).is_err());
        assert!(parse_share_code("not json").is_err());
        assert!(parse_share_code(r#"{"error":"bad"}"#).is_err());
    }
    #[test]
    fn builtin_presets_are_valid_and_distinct() {
        let presets = builtin_presets();
        assert!(presets.len() >= 8);
        for preset in &presets {
            preset.settings.validate().expect(preset.name);
            assert!(!preset.name.contains('\u{2014}'));
        }
        for (i, a) in presets.iter().enumerate() {
            for b in &presets[i + 1..] {
                assert_ne!(a.id, b.id);
                assert_ne!(a.settings, b.settings, "{} and {} look the same", a.name, b.name);
            }
        }
        assert_eq!(presets[0].settings, Crosshair::default());
        assert_eq!(presets[1].settings, Crosshair::game_defaults());
    }

    #[test]
    fn a_console_line_round_trips() {
        let mut crosshair = Crosshair::default();
        crosshair.pip_gap = 11;
        crosshair.dot_opacity = 0.35;
        crosshair.color_b = 200;
        crosshair.disable_hero_specific_crosshairs = true;
        let line = crosshair.console_line();
        match parse_import(&line, &Crosshair::game_defaults()).unwrap() {
            Import::Settings(parsed) => assert_eq!(parsed, crosshair),
            other => panic!("expected settings, got {other:?}"),
        }
        // The multi line autoexec form reads the same.
        let block = apply_to_autoexec("", &crosshair).unwrap();
        assert_eq!(parse_console(&block, &Crosshair::game_defaults()).unwrap(), crosshair);
    }

    #[test]
    fn a_partial_paste_keeps_the_rest_and_skips_other_commands() {
        let current = Crosshair::default();
        let parsed = parse_console(
            "bind F1 disconnect; citadel_crosshair_pip_gap \"9\" // wide\nfps_max 240",
            &current,
        )
        .unwrap();
        assert_eq!(parsed.pip_gap, 9);
        assert_eq!(parsed.pip_height, current.pip_height);
    }

    #[test]
    fn vcfg_lines_and_the_legacy_border_import() {
        let text = "\"citadel_crosshair_pip_gap\"\t\t\"7\"\n\"citadel_crosshair_pip_border\"\t\"false\"";
        let parsed = parse_console(text, &Crosshair::default()).unwrap();
        assert_eq!(parsed.pip_gap, 7);
        assert_eq!(parsed.pip_outline_border, 0);
    }

    #[test]
    fn import_refuses_bad_values_and_empty_pastes() {
        let current = Crosshair::default();
        assert!(parse_import("", &current).is_err());
        assert!(parse_import("   ", &current).is_err());
        assert!(parse_import("bind F1 disconnect", &current).is_err());
        assert!(parse_import("citadel_crosshair_pip_gap", &current).is_err());
        assert!(parse_import("citadel_crosshair_pip_gap abc", &current).is_err());
        assert!(parse_import("citadel_crosshair_pip_gap 2.5", &current).is_err());
        assert!(parse_import("citadel_crosshair_pip_gap 999", &current).is_err());
        assert!(parse_import("citadel_crosshair_pip_opacity 1.5", &current).is_err());
        assert!(parse_import("citadel_crosshair_pip_gap_static maybe", &current).is_err());
        assert!(parse_import("citadel_crosshair_color_r -1", &current).is_err());
        assert!(parse_import("citadel_crosshair_pip_gap NaN", &current).is_err());
        let long = "citadel_crosshair_pip_gap 5; ".repeat(1000);
        assert!(parse_import(&long, &current).is_err());
    }

    #[test]
    fn share_codes_are_recognised_by_shape() {
        assert_eq!(
            parse_import("  DL.AQAiFvB_AQK4NAL5 ", &Crosshair::default()).unwrap(),
            Import::Code("DL.AQAiFvB_AQK4NAL5".to_string())
        );
        assert!(parse_import("DL.short", &Crosshair::default()).is_err());
        assert!(parse_import("DL.AQAiFvB_AQK4NAL5<script>", &Crosshair::default()).is_err());
        assert!(!is_share_code("XL.AQAiFvB_AQK4NAL5"));
        assert!(is_share_code("DL.AQAiFvB_AQK4NAL5"));
    }

    #[test]
    fn a_share_code_reply_becomes_settings() {
        let mut current = Crosshair::default();
        current.disable_hero_specific_crosshairs = true;
        let reply = r#"{"pip_gap":12,"pip_gap_static":false,"pip_height":20,"pip_width":3,
            "pip_opacity":0.7000000001,"dot_size":6,"dot_opacity":1,"color_r":10,"color_g":20,
            "color_b":30,"unknown":"x"}"#;
        let parsed = crosshair_from_share_reply(reply, &current).unwrap();
        assert_eq!(parsed.pip_gap, 12);
        assert!(!parsed.pip_gap_static);
        assert_eq!(parsed.pip_opacity, 0.7);
        assert_eq!((parsed.color_r, parsed.color_g, parsed.color_b), (10, 20, 30));
        assert!(parsed.disable_hero_specific_crosshairs);
        assert!(crosshair_from_share_reply(r#"{"pip_gap":500}"#, &current).is_err());
        assert!(crosshair_from_share_reply(r#"{"error":"bad"}"#, &current).is_err());
        assert!(crosshair_from_share_reply("[]", &current).is_err());
    }

    #[test]
    fn preset_names_are_cleaned_and_unique() {
        assert_eq!(clean_preset_name("  My \t aim\n ").unwrap(), "My aim");
        assert!(clean_preset_name("   ").is_err());
        assert!(clean_preset_name(&"x".repeat(PRESET_NAME_MAX + 1)).is_err());

        let mut presets = Vec::new();
        let first = add_preset(&mut presets, "Tight", &Crosshair::default()).unwrap();
        assert!(add_preset(&mut presets, "tight", &Crosshair::default()).is_err());
        let second = add_preset(&mut presets, "Wide", &Crosshair::game_defaults()).unwrap();
        assert_ne!(first, second);
        assert!(rename_preset(&mut presets, &second, "TIGHT").is_err());
        rename_preset(&mut presets, &second, "Wider").unwrap();
        rename_preset(&mut presets, &second, "wider").unwrap();
        assert_eq!(presets[1].name, "wider");
        assert!(rename_preset(&mut presets, "missing", "Name").is_err());
        let mut bad = Crosshair::default();
        bad.pip_gap = -1;
        assert!(add_preset(&mut presets, "Bad", &bad).is_err());
        assert!(remove_preset(&mut presets, &first));
        assert!(!remove_preset(&mut presets, &first));
        assert_eq!(presets.len(), 1);
    }

    #[test]
    fn presets_persist_atomically_and_bad_entries_are_dropped() {
        let dir = std::env::temp_dir().join(format!("sigil-presets-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join(PRESETS_FILE);
        assert!(load_presets(&path).unwrap().is_empty());

        let mut presets = Vec::new();
        add_preset(&mut presets, "One", &Crosshair::default()).unwrap();
        add_preset(&mut presets, "Two", &Crosshair::game_defaults()).unwrap();
        save_presets(&path, &presets).unwrap();
        assert_eq!(load_presets(&path).unwrap(), presets);
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty());

        // A Grimoire file: extra fields, a legacy entry with missing settings, a bad entry.
        let grimoire = r#"{"presets":[
            {"id":"preset_1","name":"Mine","settings":{"pipGap":9,"pipBorder":true},"thumbnail":"data:","createdAt":"x"},
            {"id":"preset_2","name":"Broken","settings":{"pipGap":-4}},
            {"id":"preset_1","name":"Dupe","settings":{}},
            {"name":"No id"},
            {"id":"preset_3","name":"   ","settings":{}}
        ],"activePresetId":"preset_1"}"#;
        let parsed = parse_presets(grimoire).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name, "Mine");
        assert_eq!(parsed[0].settings.pip_gap, 9);
        assert_eq!(parsed[0].settings.pip_height, Crosshair::default().pip_height);
        assert!(parse_presets("not json").is_err());
        assert_eq!(parse_presets("[]").unwrap(), Vec::new());

        std::fs::write(&path, "{ broken").unwrap();
        assert!(load_presets(&path).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn remove_from_install_keeps_other_lines_and_backs_up() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let root = std::env::temp_dir().join(format!(
            "dah-crosshair-remove-{}-{nanos}",
            std::process::id()
        ));
        assert!(remove_from_install(&root).unwrap().is_none());
        let existing = "fps_max 144\n";
        crate::autoexec::write_autoexec(&root, existing, true).unwrap();
        assert!(remove_from_install(&root).unwrap().is_none());
        assert!(!applied_in_install(&root));
        let applied = apply_to_autoexec(existing, &Crosshair::default()).unwrap();
        crate::autoexec::write_autoexec(&root, &applied, true).unwrap();
        assert!(applied_in_install(&root));
        let result = remove_from_install(&root).unwrap().expect("a write");
        let backup = result.backup.expect("a backup");
        assert_eq!(std::fs::read_to_string(backup).unwrap(), applied);
        let now = crate::autoexec::read_autoexec(&root).unwrap().unwrap();
        assert!(now.contains("fps_max 144"));
        assert!(!now.contains(AUTOEXEC_BEGIN));
        assert!(!applied_in_install(&root));
        std::fs::remove_dir_all(root).ok();
    }
}
