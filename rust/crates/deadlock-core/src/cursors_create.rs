//! Cursor packs from the player's own images: encode RGBA art the way Deadlock's stock cursors
//! are stored, guess where each one clicks, and write the `cursor.res` that holds the hotspots.
//!
//! Ported from Grimoire's unreleased `feat/cursor-create` branch (MIT): `src/lib/cursorBmp.ts`
//! ([`encode_cursor_bmp`]), `src/lib/cursorHotspot.ts` ([`detect_hotspot`]) and the
//! `parseCursorRes` / `formatCursorRes` pair in `electron/main/services/cursorFiles.ts`.
//!
//! [`create_pack`] stores the result as an ordinary cursor pack in Sigil's app data; applying it
//! is the Locker's usual player-triggered Apply. Nothing here writes to a game install.

use std::collections::BTreeMap;
use std::path::Path;

use crate::cursors::{CursorPack, import_contents};
use crate::error::Result;

/// The stock cursors a pack can replace, by file name, with what each is for.
pub const STOCK_CURSORS: [(&str, &str); 4] = [
    ("cursor.bmp", "Pointer"),
    ("cursor_ping.bmp", "Ping"),
    ("cursor_shop.bmp", "Shop"),
    ("cursor_commend.bmp", "Commend"),
];

/// The edge a built cursor is scaled to fit when the stock file's size is not known.
pub const DEFAULT_EDGE: u32 = 32;

const HEADER_BYTES: usize = 54;
const ALPHA_MIN: u8 = 128;
/// No hull corner sharper than this means round art: aim with its middle.
const BLUNT_DEG: f64 = 130.0;
/// Penalty for a corner at the far bottom-right against one at the top-left.
const TOP_LEFT_BIAS_DEG: f64 = 180.0;

/// A click point in image pixels, from the top-left.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Hotspot {
    pub x: i32,
    pub y: i32,
}

/// One built cursor: the stock file it replaces and its art.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltCursor {
    /// A stock name from [`STOCK_CURSORS`], such as `cursor_ping.bmp`.
    pub file_name: String,
    pub width: u32,
    pub height: u32,
    /// Top-down RGBA rows.
    pub rgba: Vec<u8>,
    pub hotspot: Hotspot,
}

/// Encode RGBA pixels as Deadlock stores its cursors: a 40-byte BITMAPINFOHEADER, 32 bits,
/// BI_RGB, bottom-up BGRA rows. SDL reads the fourth byte as alpha.
pub fn encode_cursor_bmp(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    let pixel_bytes = w * h * 4;
    let mut out = vec![0u8; HEADER_BYTES + pixel_bytes];
    out[0] = b'B';
    out[1] = b'M';
    let total = out.len() as u32;
    out[2..6].copy_from_slice(&total.to_le_bytes());
    out[10..14].copy_from_slice(&(HEADER_BYTES as u32).to_le_bytes());
    out[14..18].copy_from_slice(&40u32.to_le_bytes());
    out[18..22].copy_from_slice(&(width as i32).to_le_bytes());
    out[22..26].copy_from_slice(&(height as i32).to_le_bytes());
    out[26..28].copy_from_slice(&1u16.to_le_bytes());
    out[28..30].copy_from_slice(&32u16.to_le_bytes());
    out[34..38].copy_from_slice(&(pixel_bytes as u32).to_le_bytes());
    out[38..42].copy_from_slice(&2835i32.to_le_bytes());
    out[42..46].copy_from_slice(&2835i32.to_le_bytes());
    for y in 0..h {
        let row = HEADER_BYTES + (h - 1 - y) * w * 4;
        for x in 0..w {
            let src = (y * w + x) * 4;
            let dst = row + x * 4;
            if src + 3 >= rgba.len() {
                continue;
            }
            out[dst] = rgba[src + 2];
            out[dst + 1] = rgba[src + 1];
            out[dst + 2] = rgba[src];
            out[dst + 3] = rgba[src + 3];
        }
    }
    out
}

#[derive(Clone, Copy, PartialEq)]
struct Point {
    x: f64,
    y: f64,
}

fn cross(o: Point, a: Point, b: Point) -> f64 {
    (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x)
}

fn convex_hull(points: &[Point]) -> Vec<Point> {
    let mut sorted = points.to_vec();
    sorted.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    let half = |pts: &mut dyn Iterator<Item = Point>| {
        let mut out: Vec<Point> = Vec::new();
        for p in pts {
            while out.len() >= 2 && cross(out[out.len() - 2], out[out.len() - 1], p) <= 0.0 {
                out.pop();
            }
            out.push(p);
        }
        out.pop();
        out
    };
    let mut hull = half(&mut sorted.iter().copied());
    hull.extend(half(&mut sorted.iter().rev().copied()));
    hull
}

/// The point `dist` along the hull outline from vertex `i`, walking in `dir`.
fn along_hull(hull: &[Point], i: usize, dir: isize, dist: f64) -> Point {
    let n = hull.len() as isize;
    let mut cur = hull[i];
    let mut j = i as isize;
    let mut left = dist;
    for _ in 0..hull.len() {
        j = (j + dir + n) % n;
        let next = hull[j as usize];
        let len = (next.x - cur.x).hypot(next.y - cur.y);
        if len >= left && len > 0.0 {
            return Point { x: cur.x + (next.x - cur.x) * left / len, y: cur.y + (next.y - cur.y) * left / len };
        }
        left -= len;
        cur = next;
    }
    cur
}

/// Guess where a cursor image clicks: the sharpest corner of its outline, leaning top-left like
/// the stock pointers, or the middle of a reticle or of art with no corner. Art with no
/// transparency gets the stock 0, 0.
pub fn detect_hotspot(width: u32, height: u32, rgba: &[u8]) -> Hotspot {
    let (w, h) = (width as usize, height as usize);
    if rgba.len() < w * h * 4 || w == 0 || h == 0 {
        return Hotspot::default();
    }
    let mut mask = vec![false; w * h];
    let mut edges = Vec::new();
    let mut opaque = 0usize;
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (w, h, 0usize, 0usize);
    for y in 0..h {
        let (mut left, mut right) = (None, 0usize);
        for x in 0..w {
            if rgba[(y * w + x) * 4 + 3] < ALPHA_MIN {
                continue;
            }
            mask[y * w + x] = true;
            opaque += 1;
            left.get_or_insert(x);
            right = x;
        }
        let Some(left) = left else { continue };
        edges.push(Point { x: left as f64, y: y as f64 });
        edges.push(Point { x: right as f64, y: y as f64 });
        min_x = min_x.min(left);
        max_x = max_x.max(right);
        min_y = min_y.min(y);
        max_y = y;
    }
    if opaque == 0 || opaque == w * h {
        return Hotspot::default();
    }
    let cx = (min_x + max_x) as f64 / 2.0;
    let cy = (min_y + max_y) as f64 / 2.0;
    let center = Hotspot { x: cx.round() as i32, y: cy.round() as i32 };
    // Reticles look the same turned a quarter.
    let mut symmetric = 0usize;
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            if !mask[y * w + x] {
                continue;
            }
            let rx = (cx - (y as f64 - cy)).round();
            let ry = (cy + (x as f64 - cx)).round();
            if rx >= 0.0 && (rx as usize) < w && ry >= 0.0 && (ry as usize) < h && mask[ry as usize * w + rx as usize] {
                symmetric += 1;
            }
        }
    }
    if symmetric as f64 / opaque as f64 >= 0.85 {
        return center;
    }
    let hull = convex_hull(&edges);
    if hull.len() < 3 {
        return center;
    }
    let reach = (0.15 * (max_x - min_x).max(max_y - min_y) as f64).max(2.0);
    let span = ((max_x - min_x) + (max_y - min_y)).max(1) as f64;
    let mut best = center;
    let mut best_score = f64::INFINITY;
    let mut sharpest = 180.0f64;
    for (i, v) in hull.iter().enumerate() {
        let a = along_hull(&hull, i, -1, reach);
        let b = along_hull(&hull, i, 1, reach);
        let denom = (a.x - v.x).hypot(a.y - v.y) * (b.x - v.x).hypot(b.y - v.y);
        if denom == 0.0 {
            continue;
        }
        let cos = ((a.x - v.x) * (b.x - v.x) + (a.y - v.y) * (b.y - v.y)) / denom;
        let angle = cos.clamp(-1.0, 1.0).acos().to_degrees();
        sharpest = sharpest.min(angle);
        let score = angle + TOP_LEFT_BIAS_DEG * ((v.x - min_x as f64) + (v.y - min_y as f64)) / span;
        if score < best_score {
            best_score = score;
            best = Hotspot { x: v.x as i32, y: v.y as i32 };
        }
    }
    if sharpest <= BLUNT_DEG { best } else { center }
}

/// The hotspots in a `cursor.res`, by lowercase cursor name (the BMP's stem). The game uses
/// 0, 0 for a cursor the file does not list.
pub fn parse_cursor_res(text: &str) -> BTreeMap<String, Hotspot> {
    let body: String = text.lines().map(|line| line.split("//").next().unwrap_or("")).collect::<Vec<_>>().join("\n");
    let entry = regex::Regex::new(r#""?([\w.-]+)"?\s*\{([^{}]*)\}"#).expect("static regex");
    let hotx = regex::Regex::new(r#"(?i)"hotx"\s+"(-?\d+)""#).expect("static regex");
    let hoty = regex::Regex::new(r#"(?i)"hoty"\s+"(-?\d+)""#).expect("static regex");
    let mut out = BTreeMap::new();
    for caps in entry.captures_iter(&body) {
        let read = |re: &regex::Regex| re.captures(&caps[2]).and_then(|c| c[1].parse().ok()).unwrap_or(0);
        out.insert(caps[1].to_lowercase(), Hotspot { x: read(&hotx), y: read(&hoty) });
    }
    out
}

/// Write a `cursor.res` holding these hotspots.
pub fn format_cursor_res(hotspots: &BTreeMap<String, Hotspot>) -> String {
    let entries: Vec<String> = hotspots
        .iter()
        .map(|(name, spot)| format!("\t{name}\n\t{{\n\t\t\"hotx\"\t\t\"{}\"\n\t\t\"hoty\"\t\t\"{}\"\n\t}}\n", spot.x, spot.y))
        .collect();
    format!("\"resource/cursor/cursor.res\"\n{{\n{}}}\n", entries.join("\n"))
}

/// The files of a pack built from `images`. The `cursor.res` starts from the stock one
/// (`stock_res`), so cursors left alone keep their hotspots.
pub fn build_files(stock_res: Option<&str>, images: &[BuiltCursor]) -> BTreeMap<String, Vec<u8>> {
    let mut hotspots = parse_cursor_res(stock_res.unwrap_or(""));
    let mut files = BTreeMap::new();
    for image in images {
        let name = image.file_name.to_ascii_lowercase();
        let Some(stem) = name.strip_suffix(".bmp") else { continue };
        if !crate::cursors::is_cursor_file_name(&name) {
            continue;
        }
        files.insert(name.clone(), encode_cursor_bmp(image.width, image.height, &image.rgba));
        hotspots.insert(stem.to_string(), image.hotspot);
    }
    if !files.is_empty() {
        files.insert("cursor.res".to_string(), format_cursor_res(&hotspots).into_bytes());
    }
    files
}

/// Store `images` as a new cursor pack named `name`. App data only; apply it from the Locker.
pub fn create_pack(store: &Path, stock_res: Option<&str>, name: &str, images: &[BuiltCursor]) -> Result<CursorPack> {
    import_contents(store, name, build_files(stock_res, images))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An arrow: a right triangle with its sharp tip at the top-left.
    fn arrow(edge: u32) -> Vec<u8> {
        let mut rgba = vec![0u8; (edge * edge * 4) as usize];
        for y in 0..edge {
            for x in 0..=y / 2 {
                let i = ((y * edge + x) * 4) as usize;
                rgba[i..i + 4].copy_from_slice(&[255, 255, 255, 255]);
            }
        }
        rgba
    }

    #[test]
    fn the_bmp_is_bottom_up_bgra_with_alpha() {
        let rgba = [1, 2, 3, 4, 5, 6, 7, 8]; // a 1x2 column, top pixel first
        let bmp = encode_cursor_bmp(1, 2, &rgba);
        assert_eq!(&bmp[..2], b"BM");
        assert_eq!(u16::from_le_bytes([bmp[28], bmp[29]]), 32);
        // The bottom row comes first: pixel 2, as BGRA.
        assert_eq!(&bmp[54..58], &[7, 6, 5, 8]);
        assert_eq!(&bmp[58..62], &[3, 2, 1, 4]);
    }

    #[test]
    fn an_arrow_clicks_at_its_tip_and_a_ring_at_its_middle() {
        assert_eq!(detect_hotspot(32, 32, &arrow(32)), Hotspot { x: 0, y: 0 });
        let mut ring = vec![0u8; 32 * 32 * 4];
        for y in 0..32i32 {
            for x in 0..32i32 {
                let d = ((x - 16).pow(2) + (y - 16).pow(2)) as f64;
                if (100.0..196.0).contains(&d) {
                    let i = ((y * 32 + x) * 4) as usize;
                    ring[i + 3] = 255;
                }
            }
        }
        let spot = detect_hotspot(32, 32, &ring);
        assert!((spot.x - 16).abs() <= 1 && (spot.y - 16).abs() <= 1, "{spot:?}");
        assert_eq!(detect_hotspot(2, 2, &[255; 16]), Hotspot::default(), "no transparency");
    }

    #[test]
    fn cursor_res_round_trips_and_keeps_stock_hotspots() {
        let stock = "\"resource/cursor/cursor.res\"\n{\n\tcursor_commend\n\t{\n\t\t\"hotx\"\t\t\"12\"\n\t\t\"hoty\"\t\t\"3\" // tip\n\t}\n}\n";
        let parsed = parse_cursor_res(stock);
        assert_eq!(parsed.get("cursor_commend"), Some(&Hotspot { x: 12, y: 3 }));
        let built = BuiltCursor {
            file_name: "cursor.bmp".into(),
            width: 32,
            height: 32,
            rgba: arrow(32),
            hotspot: Hotspot { x: 1, y: 2 },
        };
        let files = build_files(Some(stock), &[built]);
        assert!(files.contains_key("cursor.bmp"));
        let res = parse_cursor_res(std::str::from_utf8(&files["cursor.res"]).unwrap());
        assert_eq!(res.get("cursor"), Some(&Hotspot { x: 1, y: 2 }));
        assert_eq!(res.get("cursor_commend"), Some(&Hotspot { x: 12, y: 3 }), "stock kept");
    }

    #[test]
    fn a_created_pack_is_stored_like_an_import() {
        let store = std::env::temp_dir().join(format!("sigil-cursor-create-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&store);
        let built = BuiltCursor {
            file_name: "cursor_ping.bmp".into(),
            width: 32,
            height: 32,
            rgba: arrow(32),
            hotspot: Hotspot::default(),
        };
        let pack = create_pack(&store, None, "Mine", &[built]).unwrap();
        assert_eq!(pack.files, vec!["cursor.res".to_string(), "cursor_ping.bmp".to_string()]);
        assert!(create_pack(&store, None, "Empty", &[]).is_err());
        let _ = std::fs::remove_dir_all(&store);
    }
}
