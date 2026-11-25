//! The local matcher over the catalog search index, a port of the Worker's reference
//! (`services/telemetry/src/catalog/match.ts`, algorithm in ADR 0018). Nothing typed leaves the
//! PC: this runs over the downloaded index only.
//!
//! The port follows the reference step by step, with the owner's adjustments (2026-11-25):
//!
//! - Generated terms (classes 4 and 5) shorter than 3 characters are ignored ("hd", "cs").
//! - Class 4 (verified reference, character, franchise, alias and theme terms) counts fully at
//!   the reference's 2.5. Class 5 (the generic "visual" terms, words like "model" or "gun")
//!   counts for little. Unknown classes (6, 7) are ignored.
//! - Title matches always outrank other matches: hits sort first by how many query words the
//!   title matches, then by score, then by likes when the card is known, then by id.

use super::{CatalogIndex, PACK};

/// Field weights, as the reference names them.
pub const WEIGHT_TITLE: f32 = 3.0;
pub const WEIGHT_TERM: f32 = 2.5;
pub const WEIGHT_HERO: f32 = 2.0;
pub const WEIGHT_CATEGORY: f32 = 1.5;
pub const WEIGHT_AUTHOR: f32 = 1.5;
pub const WEIGHT_DESCRIPTION: f32 = 1.0;
/// A generic visual term ("model", "gun", "physics"): a fifth of a verified reference.
pub const WEIGHT_VISUAL: f32 = 0.5;
/// Generated terms shorter than this are ignored.
pub const MIN_TERM_CHARS: usize = 3;

/// Field classes in the packed ids (`stringId * PACK + class`).
pub const CLS_DESCRIPTION: u8 = 0;
pub const CLS_CATEGORY: u8 = 1;
pub const CLS_AUTHOR: u8 = 2;
pub const CLS_HERO: u8 = 3;
/// A verified search term: reference, character, franchise, alias or theme.
pub const CLS_TERM: u8 = 4;
/// A verified generic visual term.
pub const CLS_VISUAL: u8 = 5;

const STOP: [&str; 14] = [
    "the", "a", "an", "of", "for", "and", "mod", "mods", "with", "der", "die", "das", "und", "fur",
];

/// The weight a class carries, or None when the field is not used.
fn class_weight(class: u8, text: &str) -> Option<f32> {
    let generated = class == CLS_TERM || class == CLS_VISUAL;
    if generated && text.chars().count() < MIN_TERM_CHARS {
        return None;
    }
    match class {
        CLS_DESCRIPTION => Some(WEIGHT_DESCRIPTION),
        CLS_CATEGORY => Some(WEIGHT_CATEGORY),
        CLS_AUTHOR => Some(WEIGHT_AUTHOR),
        CLS_HERO => Some(WEIGHT_HERO),
        CLS_TERM => Some(WEIGHT_TERM),
        CLS_VISUAL => Some(WEIGHT_VISUAL),
        _ => None,
    }
}

/// Accented Latin letters to their base letter, as NFKD followed by dropping combining marks
/// does for the letters GameBanana titles use. Letters with no decomposition (`ø`, `ß`, `æ`)
/// are left alone and become spaces, as in the reference.
fn fold_letter(c: char) -> Option<&'static str> {
    const TABLE: [(&str, &str); 26] = [
        ("ÀÁÂÃÄÅàáâãäåĀāĂăĄą", "a"),
        ("", "b"),
        ("ÇçĆćĈĉĊċČč", "c"),
        ("Ďď", "d"),
        ("ÈÉÊËèéêëĒēĔĕĖėĘęĚě", "e"),
        ("", "f"),
        ("ĜĝĞğĠġĢģ", "g"),
        ("Ĥĥ", "h"),
        ("ÌÍÎÏìíîïĨĩĪīĬĭĮįİ", "i"),
        ("Ĵĵ", "j"),
        ("Ķķ", "k"),
        ("ĹĺĻļĽľĿŀ", "l"),
        ("", "m"),
        ("ÑñŃńŅņŇň", "n"),
        ("ÒÓÔÕÖòóôõöŌōŎŏŐő", "o"),
        ("", "p"),
        ("", "q"),
        ("ŔŕŖŗŘř", "r"),
        ("ŚśŜŝŞşŠš", "s"),
        ("ŢţŤť", "t"),
        ("ÙÚÛÜùúûüŨũŪūŬŭŮůŰűŲų", "u"),
        ("", "v"),
        ("Ŵŵ", "w"),
        ("", "x"),
        ("ÝýÿŶŷŸ", "y"),
        ("ŹźŻżŽž", "z"),
    ];
    TABLE
        .iter()
        .find(|(letters, _)| letters.contains(c))
        .map(|(_, base)| *base)
}

/// Lower case, accents dropped, `&` read as "and", everything but `a-z0-9` to spaces, spaces
/// collapsed. The same rule the Worker applies to every string in the index.
pub fn normalize(text: &str) -> String {
    let mut wide = String::with_capacity(text.len());
    for c in text.chars() {
        // Full-width ASCII (NFKD maps it to the plain form).
        let c = match c as u32 {
            0xFF01..=0xFF5E => char::from_u32(c as u32 - 0xFEE0).unwrap_or(' '),
            _ => c,
        };
        if c.is_ascii() {
            match c {
                '&' => wide.push_str(" and "),
                'A'..='Z' => wide.push(c.to_ascii_lowercase()),
                'a'..='z' | '0'..='9' => wide.push(c),
                _ => wide.push(' '),
            }
        } else if let Some(base) = fold_letter(c) {
            wide.push_str(base);
        } else {
            wide.push(' ');
        }
    }
    wide.split_ascii_whitespace().collect::<Vec<_>>().join(" ")
}

/// True when `a` and `b` are at most one edit apart: insert, delete, substitute, or swap two
/// neighbours.
pub fn within_one_edit(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a == b {
        return true;
    }
    if a.len().abs_diff(b.len()) > 1 {
        return false;
    }
    let (mut i, mut j, mut edits) = (0, 0, 0);
    while i < a.len() && j < b.len() {
        if a[i] == b[j] {
            i += 1;
            j += 1;
            continue;
        }
        edits += 1;
        if edits > 1 {
            return false;
        }
        if a.len() > b.len() {
            i += 1;
        } else if b.len() > a.len() {
            j += 1;
        } else if a.get(i + 1) == Some(&b[j]) && Some(&a[i]) == b.get(j + 1) {
            // A swap of two neighbours counts as one edit.
            i += 2;
            j += 2;
        } else {
            i += 1;
            j += 1;
        }
    }
    edits + (a.len() - i) + (b.len() - j) <= 1
}

/// How well one query word matches one token: exact 1.0, a prefix of 3 or more letters 0.8,
/// one edit away for words of 4 or more letters 0.7, else 0.
pub fn token_match(query: &str, token: &str) -> f32 {
    if query == token {
        1.0
    } else if query.len() >= 3 && token.starts_with(query) {
        0.8
    } else if query.len() >= 4 && token.len() >= 4 && within_one_edit(query, token) {
        0.7
    } else {
        0.0
    }
}

/// A string's words and its spaceless form.
struct Text {
    tokens: Vec<Box<str>>,
    joined: Box<str>,
}

impl Text {
    fn new(text: &str) -> Text {
        Text {
            tokens: text.split(' ').filter(|t| !t.is_empty()).map(Box::from).collect(),
            joined: text.replace(' ', "").into_boxed_str(),
        }
    }

    /// The reference's `unitQuality`: a word may also match the field written without spaces.
    fn quality(&self, word: &str) -> f32 {
        let mut best = if word.len() >= 4 && self.tokens.len() > 1 {
            token_match(word, &self.joined)
        } else {
            0.0
        };
        for token in &self.tokens {
            best = best.max(token_match(word, token));
        }
        best
    }

    fn token_count(&self) -> usize {
        // The reference splits "" into one empty token.
        self.tokens.len().max(1)
    }
}

struct PreparedMod {
    id: u64,
    title: String,
    title_text: Text,
    /// (string id, weight) for every field the matcher reads.
    fields: Vec<(u32, f32)>,
}

/// One result: the mod id, its score and how many query words its title matched.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub id: u64,
    pub score: f64,
    pub title_words: usize,
}

/// An index made ready for matching: titles and strings split once, idf per string.
pub struct Matcher {
    mods: Vec<PreparedMod>,
    texts: Vec<Text>,
    idf: Vec<f32>,
}

struct Unit {
    word: String,
    required: bool,
}

impl Matcher {
    pub fn new(index: &CatalogIndex) -> Matcher {
        let texts: Vec<Text> = index.strings.iter().map(|s| Text::new(s)).collect();
        let mut df = vec![0_u32; index.strings.len()];
        let mods: Vec<PreparedMod> = index
            .mods
            .iter()
            .map(|m| {
                let title = normalize(&m.title);
                let fields: Vec<(u32, f32)> = m
                    .fields
                    .iter()
                    .filter_map(|&packed| {
                        let sid = packed / PACK;
                        let class = (packed % PACK) as u8;
                        let text = index.strings.get(sid as usize)?;
                        let weight = class_weight(class, text)?;
                        Some((sid, weight))
                    })
                    .collect();
                for (sid, _) in &fields {
                    if !index.strings[*sid as usize].is_empty() {
                        df[*sid as usize] += 1;
                    }
                }
                PreparedMod {
                    id: m.id,
                    title_text: Text::new(&title),
                    title,
                    fields,
                }
            })
            .collect();
        let n = mods.len().max(1) as f32;
        let idf = df
            .iter()
            .map(|&count| {
                let count = count as f32;
                (1.0 + (n - count + 0.5) / (count + 0.5)).ln()
            })
            .collect();
        Matcher { mods, texts, idf }
    }

    pub fn len(&self) -> usize {
        self.mods.len()
    }

    pub fn is_empty(&self) -> bool {
        self.mods.is_empty()
    }

    /// Every match for `query`, best first, at most `limit`. `likes` breaks ties when the card
    /// is known.
    pub fn search(&self, query: &str, limit: usize, likes: impl Fn(u64) -> Option<u64>) -> Vec<Hit> {
        let whole = normalize(query);
        let units: Vec<Unit> = whole
            .split(' ')
            .filter(|w| !w.is_empty())
            .map(|word| Unit {
                word: word.to_string(),
                required: !STOP.contains(&word),
            })
            .collect();
        if units.is_empty() {
            return Vec::new();
        }
        // Each string's quality for each word, computed once per query, not once per mod.
        let quality: Vec<Vec<f32>> = units
            .iter()
            .map(|unit| self.texts.iter().map(|text| text.quality(&unit.word)).collect())
            .collect();
        let mut hits = self.score_all(&units, &quality, &whole, true);
        if hits.is_empty() {
            hits = self.score_all(&units, &quality, &whole, false);
        }
        hits.sort_by(|a, b| {
            b.title_words
                .cmp(&a.title_words)
                .then(b.score.total_cmp(&a.score))
                .then_with(|| likes(b.id).unwrap_or(0).cmp(&likes(a.id).unwrap_or(0)))
                .then(a.id.cmp(&b.id))
        });
        hits.truncate(limit);
        hits
    }

    fn score_all(&self, units: &[Unit], quality: &[Vec<f32>], whole: &str, all: bool) -> Vec<Hit> {
        let required = units.iter().filter(|u| u.required).count();
        let mut hits = Vec::new();
        // Per unit: the winning field (None for the title, Some(field index)), its value.
        let mut best: Vec<Option<(Option<usize>, f32)>> = vec![None; units.len()];
        let mut per_field: Vec<u32> = Vec::new();
        for m in &self.mods {
            let mut title_words = 0;
            per_field.clear();
            per_field.resize(m.fields.len(), 0);
            for (u, unit) in units.iter().enumerate() {
                best[u] = None;
                let tq = m.title_text.quality(&unit.word);
                if tq > 0.0 {
                    title_words += 1;
                    best[u] = Some((None, WEIGHT_TITLE * tq));
                }
                for (f, &(sid, weight)) in m.fields.iter().enumerate() {
                    let q = quality[u][sid as usize];
                    if q == 0.0 {
                        continue;
                    }
                    // An empty string has no idf in the reference (weight 1).
                    let idf = if self.texts[sid as usize].joined.is_empty() { 1.0 } else { self.idf[sid as usize] };
                    let value = weight * q * idf;
                    if best[u].is_none_or(|(_, v)| value > v) {
                        best[u] = Some((Some(f), value));
                    }
                }
                if let Some((Some(f), _)) = best[u] {
                    per_field[f] += 1;
                }
            }
            let mut total = 0.0_f64;
            let mut covered = 0;
            let mut covered_required = 0;
            let mut in_title = 0;
            for (u, unit) in units.iter().enumerate() {
                let Some((field, value)) = best[u] else { continue };
                let partial = match field {
                    None => 1.0,
                    Some(f) => {
                        let text = &self.texts[m.fields[f].0 as usize];
                        let whole_term = text.token_count() <= 1 || within_one_edit(&unit.word, &text.joined);
                        if text.token_count() > 1 && (per_field[f] as usize) < text.token_count() && !whole_term {
                            0.6
                        } else {
                            1.0
                        }
                    }
                };
                total += f64::from(value) * partial;
                covered += 1;
                if unit.required {
                    covered_required += 1;
                }
                if field.is_none() {
                    in_title += 1;
                }
            }
            if covered == 0 || (all && covered_required < required) {
                continue;
            }
            if m.title == whole {
                total += 1000.0;
            }
            if in_title == units.len() {
                total += 30.0;
            }
            hits.push(Hit {
                id: m.id,
                score: (total * 1000.0).round() / 1000.0,
                title_words,
            });
        }
        hits
    }
}
