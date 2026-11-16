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
#[serde(rename_all = "camelCase")]
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
}
