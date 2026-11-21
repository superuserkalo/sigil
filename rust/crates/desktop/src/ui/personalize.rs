//! Personalize: the data behind custom themes, the background image, the ambient glow, sidebar
//! art and corner radius. Every part is off by default; with nothing chosen Sigil paints the
//! house look exactly.
//!
//! This module is pure: settings JSON, theme files and share codes, validation, and the image
//! import that runs on a worker. The Settings view and the shell hooks live in
//! `screens::settings::personalize`.

use std::fs;
use std::path::{Path, PathBuf};

use base64::Engine;
use rune::Color;
use serde_json::{Map, Value, json};

use super::{Palette, TOKENS, hex6, preset};

/// A theme file or pasted JSON larger than this is refused before parsing.
pub const THEME_FILE_CAP: usize = 4096;
/// A share code longer than this is refused before decoding.
pub const SHARE_CODE_CAP: usize = 512;
pub const SHARE_PREFIX: &str = "sigil-theme:";
pub const THEME_VERSION: u64 = 1;
pub const NAME_MAX: usize = 40;
/// Saved themes kept in settings.
pub const THEMES_KEEP: usize = 32;
/// Saved glows kept in settings.
pub const GLOWS_KEEP: usize = 6;
/// An image bigger than this on disk is refused before it is read.
pub const IMAGE_BYTE_CAP: u64 = 40 * 1024 * 1024;
/// Long edge of a stored background, in pixels.
pub const BACKGROUND_EDGE: u32 = 2560;
/// Long edge of stored sidebar art.
pub const ART_EDGE: u32 = 1280;
/// Folder under the app data directory for copied images.
pub const DIR: &str = "personalize";

/// A theme the user saved.
#[derive(Clone, PartialEq, Debug)]
pub struct SavedTheme {
    pub id: String,
    pub name: String,
    pub palette: Palette,
}

/// The active Personalize theme.
#[derive(Clone, PartialEq, Debug)]
pub enum ThemePick {
    Preset(&'static str),
    Mine(String),
}

impl ThemePick {
    fn id(&self) -> String {
        match self {
            ThemePick::Preset(id) => format!("preset:{id}"),
            ThemePick::Mine(id) => format!("mine:{id}"),
        }
    }

    fn from_id(id: &str, themes: &[SavedTheme]) -> Option<ThemePick> {
        if let Some(rest) = id.strip_prefix("preset:") {
            return preset(rest).map(|p| ThemePick::Preset(p.id));
        }
        let rest = id.strip_prefix("mine:")?;
        themes
            .iter()
            .any(|t| t.id == rest)
            .then(|| ThemePick::Mine(rest.to_string()))
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Background {
    /// A file name inside `<data>/personalize`. Never a path.
    pub file: Option<String>,
    pub on: bool,
    /// 0 to 1.
    pub opacity: f32,
    /// Blur in points, 0 to `BLUR_MAX`.
    pub blur: f32,
    pub sidebar: bool,
    pub topbar: bool,
    /// The image also runs under the status bar (Deadlock Mod Manager's "Include bottom bar").
    pub statusbar: bool,
}

pub const BLUR_MAX: f32 = 32.0;

impl Default for Background {
    fn default() -> Self {
        Background {
            file: None,
            on: false,
            opacity: 0.3,
            blur: 0.0,
            sidebar: false,
            topbar: false,
            statusbar: false,
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Glow {
    pub on: bool,
    /// The colour entering from the top left.
    pub from: String,
    /// The colour entering from the bottom right.
    pub to: String,
    /// 0 to 1.
    pub strength: f32,
    /// 0 to 1.
    pub spread: f32,
    /// Two-colour glows the user made, newest first.
    pub mine: Vec<(String, String)>,
}

impl Default for Glow {
    fn default() -> Self {
        Glow {
            on: false,
            from: GLOWS[0].2.to_string(),
            to: GLOWS[0].3.to_string(),
            strength: 0.5,
            spread: 0.5,
            mine: Vec::new(),
        }
    }
}

/// Glow presets: id, name, top-left colour, bottom-right colour. From Grimoire's background
/// gradient presets (MIT).
pub const GLOWS: [(&str, &str, &str, &str); 7] = [
    ("ember", "Ember", "#f97316", "#dc2626"),
    ("dusk", "Dusk", "#8b5cf6", "#ec4899"),
    ("abyss", "Abyss", "#3b82f6", "#06b6d4"),
    ("verdant", "Verdant", "#10b981", "#84cc16"),
    ("nebula", "Nebula", "#6366f1", "#f43f5e"),
    ("gold", "Gold", "#f59e0b", "#eab308"),
    ("ash", "Ash", "#94a3b8", "#64748b"),
];

/// Where a piece of sidebar art comes from.
#[derive(Clone, PartialEq, Debug, Default)]
pub enum ArtSource {
    #[default]
    Off,
    /// Sigil's own: the accent glow behind the tab, the shipped art behind a launch button.
    Default,
    /// A hero render, by display name.
    Hero(String),
    /// A file name inside `<data>/personalize`.
    Custom(String),
}

/// Which part of the image shows: a focus point (0 to 1 across and down) and a zoom.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Framing {
    pub x: f32,
    pub y: f32,
    pub zoom: f32,
}

pub const ZOOM_MAX: f32 = 3.0;

impl Default for Framing {
    fn default() -> Self {
        Framing {
            x: 0.5,
            y: 0.45,
            zoom: 1.0,
        }
    }
}

#[derive(Clone, PartialEq, Debug, Default)]
pub struct ArtSlot {
    pub source: ArtSource,
    pub framing: Framing,
}

/// The surfaces that can carry art.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Surface {
    #[default]
    Tab,
    Modded,
    Vanilla,
    /// The preview volume control in the status bar (Grimoire's "Art: volume bar").
    Volume,
}

impl Surface {
    pub const ALL: [Surface; 4] = [Surface::Tab, Surface::Modded, Surface::Vanilla, Surface::Volume];

    pub fn id(self) -> &'static str {
        match self {
            Surface::Tab => "tab",
            Surface::Modded => "modded",
            Surface::Vanilla => "vanilla",
            Surface::Volume => "volume",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Surface::Tab => "Active tab",
            Surface::Modded => "Launch modded",
            Surface::Vanilla => "Launch vanilla",
            Surface::Volume => "Volume bar",
        }
    }

    /// The art file Sigil ships for the default source (Grimoire's launch art), if any.
    pub fn default_file(self) -> Option<&'static str> {
        match self {
            Surface::Tab => None,
            Surface::Modded => Some("launch-modded-bg.webp"),
            Surface::Vanilla => Some("launch-vanilla-bg.jpg"),
            Surface::Volume => Some("preview-volume-bg.jpg"),
        }
    }

    /// Default framing for the shipped art, after Grimoire's object positions.
    pub fn default_framing(self) -> Framing {
        match self {
            Surface::Tab => Framing::default(),
            Surface::Modded => Framing { x: 0.5, y: 0.45, zoom: 1.0 },
            Surface::Vanilla => Framing { x: 0.5, y: 0.48, zoom: 1.0 },
            Surface::Volume => Framing { x: 0.5, y: 0.43, zoom: 1.0 },
        }
    }
}

#[derive(Clone, PartialEq, Debug, Default)]
pub struct Art {
    pub tab: ArtSlot,
    pub modded: ArtSlot,
    pub vanilla: ArtSlot,
    pub volume: ArtSlot,
}

impl Art {
    pub fn slot(&self, surface: Surface) -> &ArtSlot {
        match surface {
            Surface::Tab => &self.tab,
            Surface::Modded => &self.modded,
            Surface::Vanilla => &self.vanilla,
            Surface::Volume => &self.volume,
        }
    }

    pub fn slot_mut(&mut self, surface: Surface) -> &mut ArtSlot {
        match surface {
            Surface::Tab => &mut self.tab,
            Surface::Modded => &mut self.modded,
            Surface::Vanilla => &mut self.vanilla,
            Surface::Volume => &mut self.volume,
        }
    }
}

/// Corner radius for the shared controls.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Radius {
    /// The house look: at most 2 points on any corner.
    #[default]
    Block,
    Square,
    Sharp,
    /// The original rounded look. Its saved id stays "default".
    Default,
    Round,
}

impl Radius {
    pub const ALL: [Radius; 5] = [Radius::Block, Radius::Square, Radius::Sharp, Radius::Default, Radius::Round];

    pub fn id(self) -> &'static str {
        match self {
            Radius::Block => "block",
            Radius::Square => "square",
            Radius::Sharp => "sharp",
            Radius::Default => "default",
            Radius::Round => "round",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Radius::Block => "Block",
            Radius::Square => "Square",
            Radius::Sharp => "Sharp",
            Radius::Default => "Soft",
            Radius::Round => "Round",
        }
    }

    pub fn scale(self) -> f32 {
        match self {
            Radius::Block => crate::ui::BLOCK_RADIUS,
            Radius::Square => 0.0,
            Radius::Sharp => 0.5,
            Radius::Default => 1.0,
            Radius::Round => 1.5,
        }
    }
}

/// Interface density for the whole shell. Text, rows and controls scale together, so nothing
/// falls off the 4 point grid inside a page.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Density {
    Compact,
    #[default]
    Default,
    Spacious,
}

impl Density {
    pub const ALL: [Density; 3] = [Density::Compact, Density::Default, Density::Spacious];

    pub fn id(self) -> &'static str {
        match self {
            Density::Compact => "compact",
            Density::Default => "default",
            Density::Spacious => "spacious",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Density::Compact => "Compact",
            Density::Default => "Default",
            Density::Spacious => "Spacious",
        }
    }

    pub fn scale(self) -> f32 {
        match self {
            Density::Compact => 0.9,
            Density::Default => 1.0,
            Density::Spacious => 1.1,
        }
    }
}

/// Everything Personalize saves, under `personalize` in settings.json.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Personal {
    pub theme: Option<ThemePick>,
    pub themes: Vec<SavedTheme>,
    pub background: Background,
    pub glow: Glow,
    pub art: Art,
    pub radius: Radius,
    pub density: Density,
}

impl Personal {
    /// The palette the active pick paints with, if any.
    pub fn palette(&self) -> Option<Palette> {
        match self.theme.as_ref()? {
            ThemePick::Preset(id) => preset(id).map(|p| p.palette()),
            ThemePick::Mine(id) => self.themes.iter().find(|t| &t.id == id).map(|t| t.palette),
        }
    }

    pub fn saved(&self, id: &str) -> Option<&SavedTheme> {
        self.themes.iter().find(|t| t.id == id)
    }

    /// A fresh id for a saved theme.
    pub fn next_id(&self) -> String {
        let n = self
            .themes
            .iter()
            .filter_map(|t| t.id.strip_prefix('t')?.parse::<u32>().ok())
            .max()
            .unwrap_or(0);
        format!("t{}", n + 1)
    }

    /// Add or replace a saved theme. Returns its id. The oldest drops past the cap.
    pub fn save_theme(&mut self, id: Option<String>, name: &str, palette: Palette) -> String {
        let name = clean_name(name).unwrap_or_else(|| "Untitled".to_string());
        if let Some(id) = id
            && let Some(t) = self.themes.iter_mut().find(|t| t.id == id)
        {
            t.name = name;
            t.palette = palette;
            return id;
        }
        let id = self.next_id();
        self.themes.push(SavedTheme {
            id: id.clone(),
            name,
            palette,
        });
        while self.themes.len() > THEMES_KEEP {
            self.themes.remove(0);
        }
        id
    }

    pub fn delete_theme(&mut self, id: &str) {
        self.themes.retain(|t| t.id != id);
        if self.theme == Some(ThemePick::Mine(id.to_string())) {
            self.theme = None;
        }
    }

    /// A name for a copy that no saved theme has yet: `Brass copy`, `Brass copy 2`.
    pub fn copy_name(&self, base: &str) -> String {
        let base = format!("{} copy", base.trim());
        let taken = |n: &str| self.themes.iter().any(|t| t.name == n);
        if !taken(&base) {
            return truncate_chars(&base, NAME_MAX);
        }
        (2..)
            .map(|i| format!("{base} {i}"))
            .find(|n| !taken(n))
            .map(|n| truncate_chars(&n, NAME_MAX))
            .unwrap_or(base)
    }

    pub fn remember_glow(&mut self, from: &str, to: &str) {
        let pair = (from.to_lowercase(), to.to_lowercase());
        self.glow.mine.retain(|p| *p != pair);
        self.glow.mine.insert(0, pair);
        self.glow.mine.truncate(GLOWS_KEEP);
    }

    /// Image files still referenced by the settings.
    pub fn files(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(file) = &self.background.file {
            out.push(file.clone());
        }
        for surface in Surface::ALL {
            if let ArtSource::Custom(file) = &self.art.slot(surface).source {
                out.push(file.clone());
            }
        }
        out
    }

    pub fn from_value(value: Option<&Value>) -> Personal {
        let d = Personal::default();
        let Some(Value::Object(map)) = value else {
            return d;
        };
        let themes: Vec<SavedTheme> = map
            .get("themes")
            .and_then(Value::as_array)
            .map(|list| {
                let mut seen = std::collections::BTreeSet::new();
                list.iter()
                    .filter_map(|t| {
                        let id = t.get("id")?.as_str()?.trim();
                        if id.is_empty() || id.len() > 16 || !seen.insert(id.to_string()) {
                            return None;
                        }
                        let name = clean_name(t.get("name")?.as_str()?)?;
                        let palette = palette_from_colors(t.get("colors")?).ok()?;
                        Some(SavedTheme {
                            id: id.to_string(),
                            name,
                            palette,
                        })
                    })
                    .take(THEMES_KEEP)
                    .collect()
            })
            .unwrap_or_default();
        let theme = map
            .get("theme")
            .and_then(Value::as_str)
            .and_then(|id| ThemePick::from_id(id, &themes));
        let bg = map.get("background");
        let f = |v: Option<&Value>, key: &str, lo: f32, hi: f32, default: f32| {
            v.and_then(|v| v.get(key))
                .and_then(Value::as_f64)
                .map(|x| x as f32)
                .filter(|x| x.is_finite())
                .map(|x| x.clamp(lo, hi))
                .unwrap_or(default)
        };
        let b = |v: Option<&Value>, key: &str, default: bool| {
            v.and_then(|v| v.get(key))
                .and_then(Value::as_bool)
                .unwrap_or(default)
        };
        fn s<'a>(v: Option<&'a Value>, key: &str) -> Option<&'a str> {
            v.and_then(|v| v.get(key)).and_then(Value::as_str)
        }
        let background = Background {
            file: s(bg, "file").filter(|n| safe_file_name(n)).map(str::to_string),
            on: b(bg, "on", d.background.on),
            opacity: f(bg, "opacity", 0.0, 1.0, d.background.opacity),
            blur: f(bg, "blur", 0.0, BLUR_MAX, d.background.blur),
            sidebar: b(bg, "sidebar", d.background.sidebar),
            topbar: b(bg, "topbar", d.background.topbar),
            statusbar: b(bg, "statusbar", d.background.statusbar),
        };
        let gl = map.get("glow");
        let hex_or = |key: &str, default: &str| {
            s(gl, key)
                .filter(|h| parse_hex6(h).is_some())
                .map(str::to_lowercase)
                .unwrap_or_else(|| default.to_string())
        };
        let glow = Glow {
            on: b(gl, "on", d.glow.on),
            from: hex_or("from", &d.glow.from),
            to: hex_or("to", &d.glow.to),
            strength: f(gl, "strength", 0.0, 1.0, d.glow.strength),
            spread: f(gl, "spread", 0.0, 1.0, d.glow.spread),
            mine: gl
                .and_then(|g| g.get("mine"))
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(|pair| {
                            let a = pair.get(0)?.as_str()?;
                            let z = pair.get(1)?.as_str()?;
                            (parse_hex6(a).is_some() && parse_hex6(z).is_some())
                                .then(|| (a.to_lowercase(), z.to_lowercase()))
                        })
                        .take(GLOWS_KEEP)
                        .collect()
                })
                .unwrap_or_default(),
        };
        let art_map = map.get("art");
        let slot = |key: &str| -> ArtSlot {
            let v = art_map.and_then(|a| a.get(key));
            let source = match s(v, "source") {
                Some("default") => ArtSource::Default,
                Some("hero") => s(v, "hero")
                    .map(str::trim)
                    .filter(|h| !h.is_empty() && h.len() <= 64)
                    .map(|h| ArtSource::Hero(h.to_string()))
                    .unwrap_or_default(),
                Some("custom") => s(v, "file")
                    .filter(|n| safe_file_name(n))
                    .map(|n| ArtSource::Custom(n.to_string()))
                    .unwrap_or_default(),
                _ => ArtSource::Off,
            };
            let fd = Framing::default();
            ArtSlot {
                source,
                framing: Framing {
                    x: f(v, "x", 0.0, 1.0, fd.x),
                    y: f(v, "y", 0.0, 1.0, fd.y),
                    zoom: f(v, "zoom", 1.0, ZOOM_MAX, fd.zoom),
                },
            }
        };
        let art = Art {
            tab: slot("tab"),
            modded: slot("modded"),
            vanilla: slot("vanilla"),
            volume: slot("volume"),
        };
        let radius = map
            .get("radius")
            .and_then(Value::as_str)
            .and_then(|id| Radius::ALL.into_iter().find(|r| r.id() == id))
            .unwrap_or_default();
        let density = map
            .get("density")
            .and_then(Value::as_str)
            .and_then(|id| Density::ALL.into_iter().find(|d| d.id() == id))
            .unwrap_or_default();
        Personal {
            theme,
            themes,
            background,
            glow,
            art,
            radius,
            density,
        }
    }

    pub fn to_value(&self) -> Value {
        let mut map = Map::new();
        map.insert(
            "theme".into(),
            self.theme.as_ref().map_or(Value::Null, |t| json!(t.id())),
        );
        map.insert(
            "themes".into(),
            Value::Array(
                self.themes
                    .iter()
                    .map(|t| json!({"id": t.id, "name": t.name, "colors": colors_value(&t.palette)}))
                    .collect(),
            ),
        );
        let bg = &self.background;
        map.insert(
            "background".into(),
            json!({
                "file": bg.file,
                "on": bg.on,
                "opacity": bg.opacity,
                "blur": bg.blur,
                "sidebar": bg.sidebar,
                "topbar": bg.topbar,
                "statusbar": bg.statusbar,
            }),
        );
        let g = &self.glow;
        map.insert(
            "glow".into(),
            json!({
                "on": g.on,
                "from": g.from,
                "to": g.to,
                "strength": g.strength,
                "spread": g.spread,
                "mine": g.mine.iter().map(|(a, z)| json!([a, z])).collect::<Vec<_>>(),
            }),
        );
        let slot = |s: &ArtSlot| {
            let mut m = Map::new();
            let (kind, hero, file) = match &s.source {
                ArtSource::Off => ("off", None, None),
                ArtSource::Default => ("default", None, None),
                ArtSource::Hero(h) => ("hero", Some(h.clone()), None),
                ArtSource::Custom(f) => ("custom", None, Some(f.clone())),
            };
            m.insert("source".into(), json!(kind));
            if let Some(hero) = hero {
                m.insert("hero".into(), json!(hero));
            }
            if let Some(file) = file {
                m.insert("file".into(), json!(file));
            }
            m.insert("x".into(), json!(s.framing.x));
            m.insert("y".into(), json!(s.framing.y));
            m.insert("zoom".into(), json!(s.framing.zoom));
            Value::Object(m)
        };
        map.insert(
            "art".into(),
            json!({
                "tab": slot(&self.art.tab),
                "modded": slot(&self.art.modded),
                "vanilla": slot(&self.art.vanilla),
                "volume": slot(&self.art.volume),
            }),
        );
        map.insert("radius".into(), json!(self.radius.id()));
        map.insert("density".into(), json!(self.density.id()));
        Value::Object(map)
    }
}

// --- names and colours -------------------------------------------------------------

fn truncate_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// A theme name: trimmed, no control characters, 1 to `NAME_MAX` characters.
pub fn clean_name(name: &str) -> Option<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().any(char::is_control) {
        return None;
    }
    Some(truncate_chars(name, NAME_MAX).trim().to_string())
}

/// `#rgb` or `#rrggbb`, the hash optional, to an opaque colour.
pub fn parse_hex6(s: &str) -> Option<Color> {
    let s = s.trim();
    let body = s.strip_prefix('#').unwrap_or(s);
    if !(body.len() == 6 || body.len() == 3) || !body.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    rune::color::hex(&format!("#{body}"))
}

/// A file name Sigil wrote: letters, digits, dash and dot, no path.
pub fn safe_file_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 96
        && !name.starts_with('.')
        && !name.contains("..")
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == '_')
}

fn colors_value(p: &Palette) -> Value {
    let mut m = Map::new();
    for (i, token) in TOKENS.iter().enumerate() {
        m.insert(token.id.into(), json!(hex6(p.get(i))));
    }
    Value::Object(m)
}

fn palette_from_colors(v: &Value) -> Result<Palette, String> {
    let Value::Object(map) = v else {
        return Err("The colours are missing.".into());
    };
    let mut p = crate::ui::PRESETS[0].palette();
    for (i, token) in TOKENS.iter().enumerate() {
        let hex = map
            .get(token.id)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("The colour {} is missing.", token.id))?;
        let c = parse_hex6(hex).ok_or_else(|| format!("The colour {} is not #rrggbb.", token.id))?;
        p.set(i, c);
    }
    Ok(p)
}

// --- theme files and share codes ---------------------------------------------------

/// A theme as a small JSON file.
pub fn theme_json(name: &str, palette: &Palette) -> String {
    let value = json!({
        "sigil_theme": THEME_VERSION,
        "name": clean_name(name).unwrap_or_else(|| "Untitled".into()),
        "colors": colors_value(palette),
    });
    serde_json::to_string_pretty(&value).unwrap_or_default() + "\n"
}

/// Read a theme file or pasted JSON. Size-capped, every colour required.
pub fn parse_theme_json(text: &str) -> Result<(String, Palette), String> {
    if text.len() > THEME_FILE_CAP {
        return Err("That theme is too large. A theme file is under 4 KB.".into());
    }
    let value: Value =
        serde_json::from_str(text).map_err(|_| "That is not a Sigil theme.".to_string())?;
    let version = value
        .get("sigil_theme")
        .and_then(Value::as_u64)
        .ok_or("That is not a Sigil theme.")?;
    if version != THEME_VERSION {
        return Err("That theme comes from a newer Sigil.".into());
    }
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .and_then(clean_name)
        .ok_or("The theme has no name.")?;
    let palette = palette_from_colors(value.get("colors").unwrap_or(&Value::Null))?;
    Ok((name, palette))
}

/// A theme as one line: `sigil-theme:` and base64url of the version, 12 sRGB triples and
/// the name.
pub fn share_code(name: &str, palette: &Palette) -> String {
    let mut bytes = vec![THEME_VERSION as u8];
    for i in 0..TOKENS.len() {
        let hex = hex6(palette.get(i));
        for k in 0..3 {
            bytes.push(u8::from_str_radix(&hex[1 + 2 * k..3 + 2 * k], 16).unwrap_or(0));
        }
    }
    bytes.extend(clean_name(name).unwrap_or_else(|| "Untitled".into()).as_bytes());
    format!(
        "{SHARE_PREFIX}{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    )
}

pub fn parse_share_code(code: &str) -> Result<(String, Palette), String> {
    let code: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    if code.len() > SHARE_CODE_CAP {
        return Err("That code is too long to be a theme.".into());
    }
    let body = code
        .strip_prefix(SHARE_PREFIX)
        .ok_or("A theme code starts with sigil-theme:")?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(body.trim_end_matches('='))
        .map_err(|_| "That theme code is damaged.".to_string())?;
    let colors = TOKENS.len() * 3;
    if bytes.len() < 1 + colors + 1 {
        return Err("That theme code is damaged.".into());
    }
    if bytes[0] as u64 != THEME_VERSION {
        return Err("That theme comes from a newer Sigil.".into());
    }
    let mut p = crate::ui::PRESETS[0].palette();
    for i in 0..TOKENS.len() {
        let rgb = &bytes[1 + 3 * i..4 + 3 * i];
        let hex = format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]);
        p.set(i, parse_hex6(&hex).ok_or("That theme code is damaged.")?);
    }
    let name = std::str::from_utf8(&bytes[1 + colors..])
        .ok()
        .and_then(clean_name)
        .ok_or("The theme has no name.")?;
    Ok((name, p))
}

/// A share code, or theme JSON, whichever the text is.
pub fn parse_theme_text(text: &str) -> Result<(String, Palette), String> {
    let trimmed = text.trim();
    if trimmed.starts_with('{') {
        parse_theme_json(trimmed)
    } else {
        parse_share_code(trimmed)
    }
}

/// A file name for an exported theme.
pub fn export_file_name(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();
    while out.contains("--") {
        out = out.replace("--", "-");
    }
    let out = out.trim_matches('-');
    format!("{}.sigil-theme.json", if out.is_empty() { "theme" } else { out })
}

// --- images ------------------------------------------------------------------------

pub fn images_dir(data: &Path) -> PathBuf {
    data.join(DIR)
}

/// Copy an image into `dir`, decoded, scaled so its long edge is at most `max_edge` and
/// re-encoded (JPEG, or PNG when it has alpha). Runs on a worker. Returns the new file name.
pub fn import_image(src: &Path, dir: &Path, prefix: &str, max_edge: u32) -> Result<String, String> {
    use sha2::Digest;
    let size = fs::metadata(src)
        .map_err(|e| format!("That image could not be read: {e}"))?
        .len();
    if size == 0 {
        return Err("That image is empty.".into());
    }
    if size > IMAGE_BYTE_CAP {
        return Err("That image is larger than 40 MB.".into());
    }
    let bytes = fs::read(src).map_err(|e| format!("That image could not be read: {e}"))?;
    let mut reader = image::ImageReader::new(std::io::Cursor::new(&bytes))
        .with_guessed_format()
        .map_err(|_| "That file is not an image Sigil can read.".to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(512 * 1024 * 1024);
    reader.limits(limits);
    let img = reader
        .decode()
        .map_err(|_| "That file is not a PNG, JPEG or WebP image Sigil can read.".to_string())?;
    let img = if img.width().max(img.height()) > max_edge {
        img.resize(max_edge, max_edge, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let mut out = Vec::new();
    let ext = if img.color().has_alpha() {
        let rgba = img.to_rgba8();
        image::codecs::png::PngEncoder::new(&mut out)
            .write_image(
                rgba.as_raw(),
                rgba.width(),
                rgba.height(),
                image::ExtendedColorType::Rgba8,
            )
            .map_err(|e| e.to_string())?;
        "png"
    } else {
        let rgb = img.to_rgb8();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90)
            .encode_image(&rgb)
            .map_err(|e| e.to_string())?;
        "jpg"
    };
    let digest = sha2::Sha256::digest(&out);
    let hash: String = digest[..8].iter().map(|b| format!("{b:02x}")).collect();
    let name = format!("{prefix}-{hash}.{ext}");
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let path = dir.join(&name);
    let partial = dir.join(format!("{name}.{}.tmp", std::process::id()));
    fs::write(&partial, &out).map_err(|e| e.to_string())?;
    fs::rename(&partial, &path).map_err(|e| {
        let _ = fs::remove_file(&partial);
        e.to_string()
    })?;
    Ok(name)
}

use image::ImageEncoder;

/// Remove copied images no setting points at any more.
pub fn prune_images(dir: &Path, keep: &[String]) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if safe_file_name(&name) && !keep.contains(&name) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Hero renders Sigil can show as art: display name, render path, chip icon path.
pub fn hero_renders() -> Vec<(String, PathBuf, Option<PathBuf>)> {
    let assets = deadlock_core::locker::LockerAssetPaths::discover();
    let Some(dir) = assets.heroes_dir else {
        return Vec::new();
    };
    let chips = assets.hero_portraits_dir.map(|d| d.join("chip-icons"));
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<(String, PathBuf, Option<PathBuf>)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let stem = name
                .strip_suffix("_Render.png")
                .or_else(|| name.strip_suffix("_Render.webp"))?;
            let display = stem.replace('_', " ");
            let key = deadlock_core::locker::hero_asset_key(&display);
            let chip = chips.as_ref().and_then(|d| {
                [format!("{key}.png"), format!("{key}.webp")]
                    .into_iter()
                    .map(|f| d.join(f))
                    .find(|p| p.is_file())
            });
            Some((display, e.path(), chip))
        })
        .collect();
    // Packs ship some heroes twice (`Doorman` and `The_Doorman`): keep one, the roster's
    // name first.
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out.sort_by_key(|(name, _, _)| deadlock_core::locker::roster_hero(name).is_none());
    let mut seen = std::collections::BTreeSet::new();
    out.retain(|(name, _, _)| {
        let key = deadlock_core::locker::hero_asset_key(name);
        seen.insert(key.trim_start_matches("the_").to_string())
    });
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Where the shipped launch art sits: beside the Locker art root.
pub fn shipped_art(file: &str) -> Option<PathBuf> {
    let assets = deadlock_core::locker::LockerAssetPaths::discover();
    let root = assets
        .general_backdrop
        .as_ref()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .or_else(|| assets.heroes_dir.as_ref().and_then(|p| p.parent().map(Path::to_path_buf)))?;
    let path = root.join(file);
    path.is_file().then_some(path)
}

/// Where a framed image sits inside a box: its drawn size and top-left offset. The image
/// covers the box at zoom 1, and the focus point stays as close to its place as the edges
/// allow.
pub fn frame(image: (f32, f32), boxed: (f32, f32), framing: Framing) -> (f32, f32, f32, f32) {
    let (iw, ih) = (image.0.max(1.0), image.1.max(1.0));
    let scale = (boxed.0 / iw).max(boxed.1 / ih) * framing.zoom.clamp(1.0, ZOOM_MAX);
    let (dw, dh) = (iw * scale, ih * scale);
    let left = -(dw - boxed.0) * framing.x.clamp(0.0, 1.0);
    let top = -(dh - boxed.1) * framing.y.clamp(0.0, 1.0);
    (dw, dh, left, top)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{PRESETS, Ui, contrast};

    fn sample() -> Personal {
        let mut p = Personal::default();
        let id = p.save_theme(None, "Night shift", PRESETS[3].palette());
        p.theme = Some(ThemePick::Mine(id));
        p.background = Background {
            file: Some("background-0011223344556677.jpg".into()),
            on: true,
            opacity: 0.5,
            blur: 8.0,
            sidebar: true,
            topbar: false,
            statusbar: true,
        };
        p.glow.on = true;
        p.remember_glow("#112233", "#445566");
        p.art.modded = ArtSlot {
            source: ArtSource::Hero("Lady Geist".into()),
            framing: Framing { x: 0.25, y: 0.5, zoom: 2.0 },
        };
        p.art.tab.source = ArtSource::Default;
        p.art.vanilla.source = ArtSource::Custom("art-vanilla-0011223344556677.png".into());
        p.art.volume = ArtSlot {
            source: ArtSource::Default,
            framing: Framing { x: 0.5, y: 0.43, zoom: 1.0 },
        };
        p.radius = Radius::Round;
        p.density = Density::Compact;
        p
    }

    #[test]
    fn settings_round_trip() {
        let p = sample();
        let back = Personal::from_value(Some(&p.to_value()));
        assert_eq!(back, p);
        let text = serde_json::to_string(&p.to_value()).unwrap();
        let again: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(Personal::from_value(Some(&again)), p);
    }

    #[test]
    fn missing_or_bad_settings_fall_back_to_off() {
        assert_eq!(Personal::from_value(None), Personal::default());
        let bad = json!({
            "theme": "mine:t9",
            "background": {"file": "../../etc/passwd", "on": true, "opacity": 9, "blur": -3},
            "glow": {"from": "red", "strength": "lots"},
            "art": {"tab": {"source": "custom", "file": "/abs/path.png"}},
            "radius": "blobby",
        });
        let p = Personal::from_value(Some(&bad));
        assert_eq!(p.theme, None, "an unknown saved theme is dropped");
        assert_eq!(p.background.file, None, "a path is never accepted as a file name");
        assert_eq!(p.background.opacity, 1.0);
        assert_eq!(p.background.blur, 0.0);
        assert_eq!(p.glow.from, Glow::default().from);
        assert_eq!(p.art.tab.source, ArtSource::Off);
        assert!(!p.background.statusbar, "the status bar stays out of the image by default");
        assert_eq!(p.art.volume.source, ArtSource::Off, "the volume bar has no art by default");
        assert_eq!(p.radius, Radius::Block);
        assert_eq!(p.density, Density::Default);
    }

    #[test]
    fn theme_json_round_trips_and_validates() {
        for preset in &PRESETS {
            let text = theme_json(preset.name, &preset.palette());
            let (name, palette) = parse_theme_json(&text).unwrap();
            assert_eq!(name, preset.name);
            for i in 0..TOKENS.len() {
                assert_eq!(hex6(palette.get(i)), preset.colors[i], "{} {}", preset.id, i);
            }
        }
        assert!(parse_theme_json("{}").is_err());
        assert!(parse_theme_json("not json").is_err());
        let mut v: Value = serde_json::from_str(&theme_json("X", &PRESETS[0].palette())).unwrap();
        v["colors"]["accent"] = json!("orange");
        assert!(parse_theme_json(&v.to_string()).is_err());
        v["colors"].as_object_mut().unwrap().remove("accent");
        assert!(parse_theme_json(&v.to_string()).is_err());
        let mut v: Value = serde_json::from_str(&theme_json("X", &PRESETS[0].palette())).unwrap();
        v["sigil_theme"] = json!(2);
        assert!(parse_theme_json(&v.to_string()).is_err());
        v["sigil_theme"] = json!(1);
        v["name"] = json!("  ");
        assert!(parse_theme_json(&v.to_string()).is_err());
        let huge = format!("{{\"pad\":\"{}\"}}", "x".repeat(THEME_FILE_CAP));
        assert!(parse_theme_json(&huge).unwrap_err().contains("too large"));
    }

    #[test]
    fn share_codes_round_trip_and_reject_damage() {
        let p = PRESETS[8].palette();
        let code = share_code("Frost wäre schön", &p);
        assert!(code.starts_with(SHARE_PREFIX));
        assert!(code.len() < SHARE_CODE_CAP);
        let (name, back) = parse_theme_text(&format!("  {code}\n")).unwrap();
        assert_eq!(name, "Frost wäre schön");
        for i in 0..TOKENS.len() {
            assert_eq!(hex6(back.get(i)), hex6(p.get(i)));
        }
        assert!(parse_share_code("sigil-theme:AAAA").is_err());
        assert!(parse_share_code("theme:abc").is_err());
        assert!(parse_share_code(&format!("{SHARE_PREFIX}{}", "A".repeat(600))).is_err());
        assert!(parse_share_code(&format!("{code}!!")).is_err());
    }

    #[test]
    fn contrast_matches_wcag() {
        assert!((contrast(Color::WHITE, Color::BLACK) - 21.0).abs() < 0.01);
        assert!((contrast(Color::WHITE, Color::WHITE) - 1.0).abs() < 0.001);
        // #777777 on white is the textbook 4.48.
        let grey = parse_hex6("#777777").unwrap();
        let r = contrast(grey, Color::WHITE);
        assert!((r - 4.48).abs() < 0.02, "{r}");
    }

    #[test]
    fn every_preset_passes_aa() {
        for preset in &PRESETS {
            let warnings = preset.palette().contrast_warnings();
            assert!(warnings.is_empty(), "{}: {:?}", preset.id, warnings);
        }
    }

    #[test]
    fn a_low_contrast_theme_warns() {
        let mut p = PRESETS[0].palette();
        p.set(5, parse_hex6("#2a2620").unwrap());
        let warnings = p.contrast_warnings();
        assert!(warnings.iter().any(|(fg, bg, _)| *fg == 5 && *bg == 0));
    }

    #[test]
    fn palette_from_the_default_look_round_trips_its_solid_tokens() {
        let u = Ui::themed(crate::ui::Theme::Dark, None);
        let p = Palette::from_ui(&u);
        let back = p.to_ui();
        assert_eq!(back.bg, u.bg);
        assert_eq!(back.ink, u.ink);
        assert_eq!(back.accent, u.accent);
        assert!(!back.light);
        assert!(PRESETS[8].palette().to_ui().light);
    }

    #[test]
    fn copies_get_free_names_and_ids() {
        let mut p = Personal::default();
        let a = p.save_theme(None, "Brass copy", PRESETS[0].palette());
        assert_eq!(p.copy_name("Brass"), "Brass copy 2");
        let b = p.save_theme(None, &p.copy_name("Brass"), PRESETS[0].palette());
        assert_ne!(a, b);
        p.theme = Some(ThemePick::Mine(a.clone()));
        p.delete_theme(&a);
        assert_eq!(p.theme, None);
        assert_eq!(p.themes.len(), 1);
    }

    #[test]
    fn framing_covers_the_box() {
        let (w, h, l, t) = frame((2000.0, 1000.0), (200.0, 40.0), Framing { x: 0.5, y: 0.5, zoom: 1.0 });
        assert!(w >= 200.0 && h >= 40.0);
        assert!(l <= 0.0 && l + w >= 200.0 && t <= 0.0 && t + h >= 40.0);
        let (w2, ..) = frame((2000.0, 1000.0), (200.0, 40.0), Framing { x: 0.5, y: 0.5, zoom: 2.0 });
        assert!((w2 - 2.0 * w).abs() < 0.01);
    }

    #[test]
    fn file_names_never_escape_the_folder() {
        assert!(safe_file_name("background-00ff.jpg"));
        assert!(!safe_file_name("../x.jpg"));
        assert!(!safe_file_name("a/b.jpg"));
        assert!(!safe_file_name("C:\\x.jpg"));
        assert!(!safe_file_name(".hidden"));
        assert!(!safe_file_name(""));
    }

    #[test]
    fn import_scales_and_copies_an_image() {
        let dir = std::env::temp_dir().join(format!("sigil-personalize-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let src = dir.join("big.png");
        let img = image::RgbImage::from_fn(3000, 1000, |x, _| image::Rgb([(x % 255) as u8, 40, 90]));
        img.save(&src).unwrap();
        let out = dir.join("out");
        let name = import_image(&src, &out, "background", 1280).unwrap();
        assert!(safe_file_name(&name) && name.ends_with(".jpg"));
        let back = image::open(out.join(&name)).unwrap();
        assert_eq!(back.width(), 1280);
        fs::write(dir.join("junk.png"), b"not an image").unwrap();
        assert!(import_image(&dir.join("junk.png"), &out, "background", 1280).is_err());
        prune_images(&out, &[]);
        assert!(fs::read_dir(&out).unwrap().next().is_none());
        let _ = fs::remove_dir_all(&dir);
    }
}
