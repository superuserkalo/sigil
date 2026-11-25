//! The local matcher over the catalog search index (ADR 0018). Nothing typed leaves the PC:
//! this runs over the downloaded index only.
//!
//! Scoring (the search review's prototype `V2`, 2026-11-25):
//!
//! - Every mod gets one token table: each word of its title and fields with the best field
//!   weight it carries (title 3, verified term 2.5, hero 2, category and author 1.5, description
//!   1, generic visual term 0.5). Adjacent title words are also indexed joined ("delta rune"
//!   as "deltarune"), and so is every multi-word field.
//! - Each token has an idf across all fields. A query word scores idf times the best weight
//!   times its match quality: exact 1, a prefix of 3 or more letters 0.8, one edit (a swap
//!   included) for words of 4 or more letters 0.7, two edits for words of 8 or more against a
//!   title, term, hero, category or author token of 7 or more 0.5.
//! - Coverage is the matched share of the query's idf mass. Stop words weigh 0.15; a word the
//!   index never saw weighs the most an idf can.
//! - Mods covering the whole query are the answer. With none, the closest matches (covering at
//!   least half, within 0.15 of the best) are returned and flagged as such. With nothing
//!   covering half, there is no answer, so the grid shows its empty state.
//! - Two typed words may be one word on the page ("counter strike" finds "counterstrike"), and
//!   one unknown typed word of 6 or more letters may be two known words ("hitsound").
//! - Text in a non-Latin script is matched as a lower-case substring of the raw title.
//! - With the optional server keys: likes lift a score by `1 + 0.12 * log10(1 + likes)`, and
//!   "nsfw", "lewd", "18", "adult" filter on the content-rating flag instead of matching text.
//!   Without them the same words are ordinary text.

use std::collections::HashMap;

use super::{CatalogIndex, PACK};

/// Field weights.
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

const STOP: [&str; 25] = [
    "the", "a", "an", "of", "for", "and", "mod", "mods", "with", "as", "over", "in", "on", "to", "de", "der", "die",
    "das", "und", "fur", "at", "by", "from", "is", "deadlock",
];
/// Query words that ask for mods with a content rating, when the index carries the flag.
pub const RATED_WORDS: [&str; 5] = ["nsfw", "lewd", "18", "adult", "r18"];
/// The share of the query a stop word stands for.
const STOP_SHARE: f32 = 0.15;
/// Coverage at or above this is a full match.
const FULL: f32 = 0.999;
/// The least coverage a closest match needs.
const HALF: f32 = 0.5;
/// Closest matches stay within this of the best one.
const BAND: f32 = 0.15;
/// How much likes lift a score.
const LIKES_LIFT: f64 = 0.12;
/// Joined title words weigh this share of the title weight.
const JOINED_SHARE: f32 = 0.9;
/// The quality of each half of an unknown word split in two.
const SPLIT_QUALITY: f32 = 0.9;
/// An exact title match always comes first.
const EXACT_TITLE: f64 = 1000.0;

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

/// True when `a` and `b` are at most two edits apart (Damerau-Levenshtein, neighbour swaps
/// count as one).
pub fn within_two_edits(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len().abs_diff(b.len()) > 2 {
        return false;
    }
    let width = b.len() + 1;
    let mut prev2 = vec![0_usize; width];
    let mut prev: Vec<usize> = (0..width).collect();
    let mut cur = vec![0_usize; width];
    for i in 1..=a.len() {
        cur[0] = i;
        let mut row_min = i;
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut v = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                v = v.min(prev2[j - 2] + 1);
            }
            cur[j] = v;
            row_min = row_min.min(v);
        }
        if row_min > 2 {
            return false;
        }
        std::mem::swap(&mut prev2, &mut prev);
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()] <= 2
}

/// True when a typed word has letters outside the Latin script (Cyrillic, CJK, Arabic and so
/// on): the tokeniser would drop them, so the word is matched against the raw title instead.
pub fn is_foreign(word: &str) -> bool {
    word.chars().any(|c| c as u32 > 0x24F && c.is_alphabetic())
}

/// One result: the mod id, its score and the share of the query it covers (0 to 1).
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub id: u64,
    pub score: f64,
    pub cover: f32,
}

/// What a search found.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Found {
    /// Best first.
    pub hits: Vec<Hit>,
    /// No mod covers the whole query: these are the closest matches.
    pub closest: bool,
    /// The query asked for rated mods ("nsfw") and only those are in `hits`.
    pub rated_only: bool,
}

struct PreparedMod {
    id: u64,
    /// The normalised title.
    title: String,
    /// The raw title in lower case, for non-Latin text.
    raw: String,
    likes: u64,
    rated: bool,
}

/// An index made ready for matching: one token table, idf per token, the mods holding each.
pub struct Matcher {
    vocab: HashMap<Box<str>, u32>,
    tokens: Vec<Box<str>>,
    df: Vec<u32>,
    /// The token appears somewhere outside a description (two-edit matches need it).
    strong: Vec<bool>,
    /// Per token: (mod index, best weight in that mod).
    postings: Vec<Vec<(u32, f32)>>,
    mods: Vec<PreparedMod>,
    /// Some mod carries the content-rating key, so the flag can be trusted.
    has_ratings: bool,
    /// Mod-kind words in the app's UI languages to the English word ("karte" to "map").
    kind_words: HashMap<String, String>,
}

/// A query word ready to score.
struct Unit {
    matches: Vec<(u32, f32)>,
    /// Two known tokens the word is written as, when it is unknown itself.
    split: Option<(u32, u32)>,
    share: f32,
}

impl Matcher {
    pub fn new(index: &CatalogIndex) -> Matcher {
        let mut m = Matcher {
            vocab: HashMap::new(),
            tokens: Vec::new(),
            df: Vec::new(),
            strong: Vec::new(),
            postings: Vec::new(),
            mods: Vec::with_capacity(index.mods.len()),
            has_ratings: index.mods.iter().any(|m| m.rated.is_some()),
            kind_words: HashMap::new(),
        };
        let mut table: HashMap<u32, f32> = HashMap::new();
        for (at, item) in index.mods.iter().enumerate() {
            table.clear();
            let title = normalize(&item.title);
            let words: Vec<&str> = title.split(' ').filter(|w| !w.is_empty()).collect();
            for word in &words {
                m.put(&mut table, word, WEIGHT_TITLE, true);
            }
            let joinable: Vec<&str> = words.iter().copied().filter(|w| *w != "and").collect();
            for pair in joinable.windows(2) {
                m.put(&mut table, &format!("{}{}", pair[0], pair[1]), WEIGHT_TITLE * JOINED_SHARE, true);
            }
            for &packed in &item.fields {
                let class = (packed % PACK) as u8;
                let Some(text) = index.strings.get((packed / PACK) as usize) else { continue };
                let Some(weight) = class_weight(class, text) else { continue };
                let parts: Vec<&str> = text.split(' ').filter(|w| !w.is_empty()).collect();
                for part in &parts {
                    m.put(&mut table, part, weight, class != CLS_DESCRIPTION);
                }
                if parts.len() > 1 {
                    m.put(&mut table, &parts.concat(), weight, true);
                }
            }
            for (&token, &weight) in &table {
                m.df[token as usize] += 1;
                m.postings[token as usize].push((at as u32, weight));
            }
            m.mods.push(PreparedMod {
                id: item.id,
                title,
                raw: item.title.to_lowercase(),
                likes: item.likes.unwrap_or(0),
                rated: item.rated == Some(true),
            });
        }
        m
    }

    fn put(&mut self, table: &mut HashMap<u32, f32>, token: &str, weight: f32, strong: bool) {
        if token.is_empty() {
            return;
        }
        let id = match self.vocab.get(token) {
            Some(&id) => id,
            None => {
                let id = self.tokens.len() as u32;
                self.vocab.insert(Box::from(token), id);
                self.tokens.push(Box::from(token));
                self.df.push(0);
                self.strong.push(false);
                self.postings.push(Vec::new());
                id
            }
        };
        let slot = table.entry(id).or_insert(0.0);
        if *slot < weight {
            *slot = weight;
        }
        if strong {
            self.strong[id as usize] = true;
        }
    }

    /// Words in the app's UI languages for mod kinds, each to the English word it means.
    /// Keys and values are normalised here.
    pub fn set_kind_words(&mut self, words: impl IntoIterator<Item = (String, String)>) {
        self.kind_words = words
            .into_iter()
            .map(|(from, to)| (normalize(&from), normalize(&to)))
            .filter(|(from, to)| !from.is_empty() && !to.is_empty() && from != to && !from.contains(' '))
            .collect();
    }

    pub fn len(&self) -> usize {
        self.mods.len()
    }

    pub fn is_empty(&self) -> bool {
        self.mods.is_empty()
    }

    /// True when the index carries the content-rating key for some mod.
    pub fn has_ratings(&self) -> bool {
        self.has_ratings
    }

    fn idf(&self, df: u32) -> f32 {
        let n = self.mods.len() as f32;
        let df = df as f32;
        (1.0 + (n - df + 0.5) / (df + 0.5)).ln()
    }

    /// Tokens a typed word matches, with the match quality.
    fn expand(&self, word: &str) -> Vec<(u32, f32)> {
        let mut out = Vec::new();
        let exact = self.vocab.get(word).copied();
        if let Some(id) = exact {
            out.push((id, 1.0));
        }
        let len = word.len();
        for (id, token) in self.tokens.iter().enumerate() {
            let id = id as u32;
            if Some(id) == exact {
                continue;
            }
            let token: &str = token;
            if len >= 3 && token.starts_with(word) {
                out.push((id, 0.8));
            } else if len >= 4 && token.len() >= 4 && within_one_edit(word, token) {
                out.push((id, 0.7));
            } else if len >= 8 && token.len() >= 7 && self.strong[id as usize] && within_two_edits(word, token) {
                out.push((id, 0.5));
            }
        }
        out
    }

    fn unit(&self, word: &str) -> Unit {
        let matches = self.expand(word);
        let exact = self.vocab.get(word).copied();
        let n = self.mods.len() as u32;
        let df = match exact {
            Some(id) => self.df[id as usize],
            None => matches.iter().map(|(id, _)| self.df[*id as usize]).sum::<u32>().min(n),
        };
        let mut idf = if matches.is_empty() { self.idf(0) } else { self.idf(df) };
        let mut split = None;
        if exact.is_none() && word.len() >= 6 {
            // A word nobody wrote that way: two known words written together.
            let mut best_df = u32::MAX;
            for i in 2..word.len() {
                let (a, b) = word.split_at(i);
                if b.len() < 3 && !b.bytes().all(|c| c.is_ascii_digit()) {
                    continue;
                }
                let (Some(&ia), Some(&ib)) = (self.vocab.get(a), self.vocab.get(b)) else { continue };
                let d = self.df[ia as usize].min(self.df[ib as usize]);
                if d < best_df {
                    best_df = d;
                    split = Some((ia, ib));
                }
            }
            if split.is_some() && matches.is_empty() {
                idf = self.idf(best_df);
            }
        }
        let share = if STOP.contains(&word) { STOP_SHARE } else { 1.0 };
        Unit {
            matches,
            split,
            share: share * idf,
        }
    }

    /// Per mod, the best weight times quality over `matches`.
    fn spread(&self, matches: &[(u32, f32)], out: &mut [f32]) {
        for &(token, quality) in matches {
            for &(at, weight) in &self.postings[token as usize] {
                let value = weight * quality;
                let slot = &mut out[at as usize];
                if value > *slot {
                    *slot = value;
                }
            }
        }
    }

    /// Every match for `query`, best first, at most `limit`.
    pub fn search(&self, query: &str, limit: usize) -> Found {
        let n = self.mods.len();
        let mut words: Vec<String> = normalize(query).split(' ').filter(|w| !w.is_empty()).map(str::to_string).collect();
        let rated_only = self.has_ratings && words.iter().any(|w| RATED_WORDS.contains(&w.as_str()));
        if rated_only {
            words.retain(|w| !RATED_WORDS.contains(&w.as_str()));
        }
        for word in &mut words {
            if let Some(english) = self.kind_words.get(word.as_str()) {
                *word = english.clone();
            }
        }
        let lower = query.to_lowercase();
        let foreign: Vec<&str> = lower.split_whitespace().filter(|w| is_foreign(w)).collect();
        if words.is_empty() && foreign.is_empty() {
            if !rated_only {
                return Found::default();
            }
            let mut hits: Vec<Hit> = self
                .mods
                .iter()
                .filter(|m| m.rated)
                .map(|m| Hit {
                    id: m.id,
                    score: m.likes as f64,
                    cover: 1.0,
                })
                .collect();
            hits.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.id.cmp(&b.id)));
            hits.truncate(limit);
            return Found {
                hits,
                closest: false,
                rated_only,
            };
        }
        let units: Vec<Unit> = words.iter().map(|w| self.unit(w)).collect();
        let idf0 = self.idf(0);
        let total: f32 = units.iter().map(|u| u.share).sum::<f32>() + foreign.len() as f32 * idf0;
        // Per unit and mod: the best weight times quality.
        let mut value = vec![vec![0.0_f32; n]; units.len()];
        let mut side_a = vec![0.0_f32; n];
        let mut side_b = vec![0.0_f32; n];
        for (u, unit) in units.iter().enumerate() {
            self.spread(&unit.matches, &mut value[u]);
            if let Some((a, b)) = unit.split {
                side_a.iter_mut().for_each(|v| *v = 0.0);
                side_b.iter_mut().for_each(|v| *v = 0.0);
                self.spread(&[(a, SPLIT_QUALITY)], &mut side_a);
                self.spread(&[(b, SPLIT_QUALITY)], &mut side_b);
                for at in 0..n {
                    if side_a[at] > 0.0 && side_b[at] > 0.0 {
                        let both = side_a[at].min(side_b[at]);
                        if both > value[u][at] {
                            value[u][at] = both;
                        }
                    }
                }
            }
        }
        // Two typed words that are one word on the page ("delta rune", "spider man").
        for at in 0..words.len().saturating_sub(1) {
            let joined = format!("{}{}", words[at], words[at + 1]);
            if joined.len() < 5 {
                continue;
            }
            let matches = self.expand(&joined);
            if matches.is_empty() {
                continue;
            }
            side_a.iter_mut().for_each(|v| *v = 0.0);
            self.spread(&matches, &mut side_a);
            for (m, &v) in side_a.iter().enumerate() {
                if v > 0.0 {
                    for u in [at, at + 1] {
                        if v > value[u][m] {
                            value[u][m] = v;
                        }
                    }
                }
            }
        }
        let whole = normalize(query);
        let mut hits: Vec<Hit> = Vec::new();
        for (at, m) in self.mods.iter().enumerate() {
            if rated_only && !m.rated {
                continue;
            }
            let (mut mass, mut score) = (0.0_f32, 0.0_f64);
            for (u, unit) in units.iter().enumerate() {
                let v = value[u][at];
                if v > 0.0 {
                    mass += unit.share * if v >= 0.5 { 1.0 } else { v * 2.0 };
                    score += f64::from(unit.share * v);
                }
            }
            for word in &foreign {
                if m.raw.contains(word) {
                    mass += idf0;
                    score += f64::from(idf0 * 3.0);
                }
            }
            if mass == 0.0 {
                continue;
            }
            if !whole.is_empty() && m.title == whole {
                score += EXACT_TITLE;
            }
            score *= 1.0 + LIKES_LIFT * (1.0 + m.likes as f64).log10();
            hits.push(Hit {
                id: m.id,
                score,
                cover: if total > 0.0 { mass / total } else { 0.0 },
            });
        }
        let full = hits.iter().any(|h| h.cover >= FULL);
        let closest = !full && !hits.is_empty();
        if full {
            hits.retain(|h| h.cover >= FULL);
        } else {
            let best = hits.iter().map(|h| h.cover).fold(0.0_f32, f32::max);
            let floor = HALF.max(best - BAND);
            hits.retain(|h| best >= HALF && h.cover >= floor);
        }
        let tier = |h: &Hit| (h.cover * 20.0).round() as i32;
        hits.sort_by(|a, b| tier(b).cmp(&tier(a)).then(b.score.total_cmp(&a.score)).then(a.id.cmp(&b.id)));
        hits.truncate(limit);
        Found {
            closest: closest && !hits.is_empty(),
            hits,
            rated_only,
        }
    }
}
