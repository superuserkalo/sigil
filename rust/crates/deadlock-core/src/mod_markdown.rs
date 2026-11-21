//! Safe Markdown for a third-party source's description.
//!
//! A source is untrusted input: its description may carry raw HTML, remote images and links
//! to schemes Sigil must never hand to the system. This reader keeps only what Sigil can draw
//! in its own text styles: paragraphs, headings, bullets, quotes and code, with bold, italic,
//! inline code, strike and https links. Raw HTML is removed, images are dropped and a link is
//! kept only when it is a public https address (the same check a source address gets). The
//! output carries byte ranges, never markup, so the UI renders text and spans and nothing can
//! execute.

/// The most blocks one description keeps. A long changelog still reads.
pub const MAX_BLOCKS: usize = 256;
/// The most styled ranges one block keeps.
pub const MAX_SPANS: usize = 512;
/// The most nested emphasis markers one inline run reads.
const MAX_DEPTH: usize = 8;
/// The longest link kept from a description.
const MAX_LINK: usize = 2048;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BlockKind {
    #[default]
    Paragraph,
    Heading(u8),
    Bullet,
    Quote,
    Code,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Block {
    pub kind: BlockKind,
    pub text: String,
    pub spans: Vec<InlineSpan>,
    pub links: Vec<LinkRange>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InlineSpan {
    pub start: u32,
    pub end: u32,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub strike: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkRange {
    pub start: u32,
    pub end: u32,
    /// A public https address, checked as the source's own address is.
    pub url: String,
}

#[derive(Debug, Clone, Copy, Default)]
struct Style {
    bold: bool,
    italic: bool,
    code: bool,
    strike: bool,
}

impl Style {
    fn marked(self) -> bool {
        self.bold || self.italic || self.code || self.strike
    }
}

#[derive(Default)]
struct Out {
    text: String,
    spans: Vec<InlineSpan>,
    links: Vec<LinkRange>,
}

impl Out {
    fn emit(&mut self, text: &str, style: Style) {
        let mut clean = String::with_capacity(text.len());
        for c in text.chars() {
            if c == '\n' || c == '\t' {
                clean.push(if c == '\t' { ' ' } else { '\n' });
            } else if c.is_control() {
                clean.push(' ');
            } else {
                clean.push(c);
            }
        }
        if clean.is_empty() {
            return;
        }
        let start = self.text.len() as u32;
        self.text.push_str(&clean);
        let end = self.text.len() as u32;
        if !style.marked() {
            return;
        }
        if let Some(last) = self.spans.last_mut()
            && last.end == start
            && last.bold == style.bold
            && last.italic == style.italic
            && last.code == style.code
            && last.strike == style.strike
        {
            last.end = end;
            return;
        }
        if self.spans.len() < MAX_SPANS {
            self.spans.push(InlineSpan {
                start,
                end,
                bold: style.bold,
                italic: style.italic,
                code: style.code,
                strike: style.strike,
            });
        }
    }

    fn link(&mut self, start: u32, end: u32, url: &str) {
        if end > start && self.links.len() < MAX_SPANS {
            self.links.push(LinkRange {
                start,
                end,
                url: url.to_string(),
            });
        }
    }
}

/// Read a source's description into blocks with byte ranges.
pub fn parse(source: &str) -> Vec<Block> {
    let normalized = source.replace("\r\n", "\n").replace('\r', "\n");
    let cleaned = strip_html_blocks(&normalized);
    let lines: Vec<&str> = cleaned.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() && out.len() < MAX_BLOCKS {
        let raw = lines[i];
        let line = raw.trim();
        if line.is_empty() {
            i += 1;
            continue;
        }
        if let Some(fence) = fence_marker(line) {
            let mut code = String::new();
            i += 1;
            while i < lines.len() {
                if lines[i].trim_start().starts_with(fence) {
                    i += 1;
                    break;
                }
                code.push_str(lines[i]);
                code.push('\n');
                i += 1;
            }
            push(&mut out, BlockKind::Code, code.trim_end(), Out::default());
            continue;
        }
        if is_thematic(line) {
            i += 1;
            continue;
        }
        if let Some(level) = heading_level(line) {
            let text = line[level..].trim();
            push_inline(&mut out, BlockKind::Heading(level as u8), text);
            i += 1;
            continue;
        }
        if line.starts_with('>') {
            let mut parts = Vec::new();
            while i < lines.len() && lines[i].trim_start().starts_with('>') {
                parts.push(lines[i].trim_start().trim_start_matches('>').trim_start());
                i += 1;
            }
            push_inline(&mut out, BlockKind::Quote, &parts.join(" "));
            continue;
        }
        if let Some(item) = list_item(line) {
            push_inline(&mut out, BlockKind::Bullet, item);
            i += 1;
            continue;
        }
        if line.starts_with('|') {
            let mut rows = Vec::new();
            while i < lines.len() && lines[i].trim().starts_with('|') {
                let row = lines[i].trim();
                if !is_table_rule(row) {
                    let cells: Vec<&str> = row
                        .trim_matches('|')
                        .split('|')
                        .map(str::trim)
                        .filter(|cell| !cell.is_empty())
                        .collect();
                    if !cells.is_empty() {
                        rows.push(cells.join("  ·  "));
                    }
                }
                i += 1;
            }
            if !rows.is_empty() {
                push_inline(&mut out, BlockKind::Paragraph, &rows.join("\n"));
            }
            continue;
        }
        let mut parts = Vec::new();
        while i < lines.len() {
            let next = lines[i].trim();
            if next.is_empty() || starts_block(next) {
                break;
            }
            parts.push(next);
            i += 1;
        }
        if !parts.is_empty() {
            push_inline(&mut out, BlockKind::Paragraph, &parts.join(" "));
        }
    }
    out
}

/// Markdown as plain text, for a line that has no span renderer.
pub fn to_text(source: &str) -> String {
    let mut out = String::new();
    let mut previous_bullet = false;
    for block in parse(source) {
        let text = block.text.trim();
        if text.is_empty() {
            continue;
        }
        if !out.is_empty() {
            if block.kind == BlockKind::Bullet && previous_bullet {
                out.push('\n');
            } else {
                out.push_str("\n\n");
            }
        }
        if block.kind == BlockKind::Bullet {
            out.push_str("• ");
        }
        out.push_str(text);
        previous_bullet = block.kind == BlockKind::Bullet;
    }
    out.trim().to_string()
}

fn push(out: &mut Vec<Block>, kind: BlockKind, text: &str, inline: Out) {
    if text.trim().is_empty() && kind != BlockKind::Code {
        return;
    }
    out.push(Block {
        kind,
        text: text.to_string(),
        spans: inline.spans,
        links: inline.links,
    });
}

fn push_inline(out: &mut Vec<Block>, kind: BlockKind, text: &str) {
    let mut inline = Out::default();
    inline_text(text, Style::default(), 0, &mut inline);
    let Out { text, spans, links } = inline;
    if text.trim().is_empty() {
        return;
    }
    out.push(Block { kind, text, spans, links });
}

fn starts_block(line: &str) -> bool {
    fence_marker(line).is_some()
        || heading_level(line).is_some()
        || line.starts_with('>')
        || line.starts_with('|')
        || list_item(line).is_some()
        || is_thematic(line)
}

fn fence_marker(line: &str) -> Option<&'static str> {
    let line = line.trim_start();
    if line.starts_with("```") {
        Some("```")
    } else if line.starts_with("~~~") {
        Some("~~~")
    } else {
        None
    }
}

fn heading_level(line: &str) -> Option<usize> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&hashes) && line[hashes..].starts_with(' ') {
        Some(hashes)
    } else {
        None
    }
}

fn is_thematic(line: &str) -> bool {
    let compact: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    compact.len() >= 3
        && (compact.chars().all(|c| c == '-')
            || compact.chars().all(|c| c == '*')
            || compact.chars().all(|c| c == '_'))
}

fn is_table_rule(line: &str) -> bool {
    let row = line.trim_matches('|');
    !row.is_empty()
        && row
            .chars()
            .all(|c| matches!(c, '|' | '-' | ':' | ' '))
        && row.contains('-')
}

fn list_item(line: &str) -> Option<&str> {
    for marker in ["- ", "* ", "+ "] {
        if let Some(rest) = line.strip_prefix(marker) {
            return Some(strip_checkbox(rest));
        }
    }
    let digits = line.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0
        && line[digits..].chars().next().is_some_and(|c| c == '.' || c == ')')
        && line[digits + 1..].starts_with(' ')
    {
        return Some(strip_checkbox(line[digits + 2..].trim_start()));
    }
    None
}

fn strip_checkbox(text: &str) -> &str {
    for mark in ["[ ] ", "[x] ", "[X] "] {
        if let Some(rest) = text.strip_prefix(mark) {
            return rest;
        }
    }
    text
}

// --- inline -----------------------------------------------------------------------------

const SPECIAL: [char; 8] = ['\\', '`', '*', '_', '~', '!', '[', '<'];

fn inline_text(src: &str, style: Style, depth: usize, out: &mut Out) {
    if depth > MAX_DEPTH {
        out.emit(src, style);
        return;
    }
    let mut i = 0;
    while i < src.len() {
        let rest = &src[i..];
        let c = rest.chars().next().unwrap_or(' ');
        match c {
            '\\' => {
                if let Some(next) = rest.chars().nth(1) {
                    out.emit(&next.to_string(), style);
                    i += 1 + next.len_utf8();
                } else {
                    i += 1;
                }
            }
            '`' => {
                if let Some(close) = rest[1..].find('`') {
                    let code = &rest[1..1 + close];
                    let mut code_style = style;
                    code_style.code = true;
                    out.emit(code, code_style);
                    i += 1 + close + 1;
                } else {
                    out.emit("`", style);
                    i += 1;
                }
            }
            '*' | '_' => {
                let marker = c.to_string();
                let doubled = format!("{marker}{marker}");
                let word_before = src[..i].chars().last().is_some_and(|c| c.is_alphanumeric());
                let marker_ok = c != '_' || !word_before;
                if marker_ok
                    && rest.starts_with(&doubled)
                    && can_open(&rest[2..])
                    && let Some(close) = rest[2..].find(&doubled)
                    && closes_cleanly(&rest[2..2 + close])
                {
                    inline_text(
                        &rest[2..2 + close],
                        Style { bold: true, ..style },
                        depth + 1,
                        out,
                    );
                    i += 2 + close + 2;
                } else if marker_ok
                    && can_open(&rest[1..])
                    && let Some(close) = rest[1..].find(c)
                    && closes_cleanly(&rest[1..1 + close])
                {
                    inline_text(
                        &rest[1..1 + close],
                        Style { italic: true, ..style },
                        depth + 1,
                        out,
                    );
                    i += 1 + close + 1;
                } else {
                    out.emit(&marker, style);
                    i += 1;
                }
            }
            '~' if rest.starts_with("~~") => {
                if can_open(&rest[2..])
                    && let Some(close) = rest[2..].find("~~")
                    && closes_cleanly(&rest[2..2 + close])
                {
                    inline_text(
                        &rest[2..2 + close],
                        Style { strike: true, ..style },
                        depth + 1,
                        out,
                    );
                    i += 2 + close + 2;
                } else {
                    out.emit("~~", style);
                    i += 2;
                }
            }
            '!' if rest.starts_with("![") => match image(rest) {
                Some(len) => i += len,
                None => {
                    out.emit("!", style);
                    i += 1;
                }
            },
            '[' => match link(rest) {
                Some((label, target, len)) => {
                    let start = out.text.len() as u32;
                    if label.is_empty() {
                        if let Some(target) = target {
                            out.emit(target, style);
                        }
                        let end = out.text.len() as u32;
                        if let Some(target) = target {
                            out.link(start, end, target);
                        }
                    } else {
                        inline_text(label, style, depth + 1, out);
                        let end = out.text.len() as u32;
                        if let Some(target) = target {
                            out.link(start, end, target);
                        }
                    }
                    i += len;
                }
                None => {
                    out.emit("[", style);
                    i += 1;
                }
            },
            '<' => {
                if let Some((text, target, len)) = angle(rest) {
                    let start = out.text.len() as u32;
                    out.emit(&text, style);
                    let end = out.text.len() as u32;
                    if let Some(target) = target {
                        out.link(start, end, &target);
                    }
                    i += len;
                } else {
                    out.emit("<", style);
                    i += 1;
                }
            }
            '&' => {
                if let Some((text, len)) = entity(rest) {
                    out.emit(&text, style);
                    i += len;
                } else {
                    out.emit("&", style);
                    i += 1;
                }
            }
            _ => {
                let end = rest
                    .char_indices()
                    .find(|(_, c)| *c == '&' || SPECIAL.contains(c))
                    .map_or(rest.len(), |(at, _)| at);
                if end == 0 {
                    out.emit(&c.to_string(), style);
                    i += c.len_utf8();
                } else {
                    out.emit(&rest[..end], style);
                    i += end;
                }
            }
        }
    }
}

fn can_open(rest: &str) -> bool {
    rest.chars().next().is_some_and(|next| !next.is_whitespace())
}

fn closes_cleanly(inner: &str) -> bool {
    !inner.is_empty()
        && !inner.starts_with(char::is_whitespace)
        && !inner.ends_with(char::is_whitespace)
}

/// `[label](target)` or `[label](<target>)`; returns the label, the safe target and bytes read.
fn link(rest: &str) -> Option<(&str, Option<&str>, usize)> {
    let close = rest[1..].find(']')? + 1;
    if rest[close + 1..].chars().next()? != '(' {
        return None;
    }
    let after = close + 2;
    let end = rest[after..].find(')')? + after;
    let raw = rest[after..end].trim();
    let target = raw
        .trim_start_matches('<')
        .trim_end_matches('>')
        .split_whitespace()
        .next()
        .filter(|target| safe_link(target).is_some());
    Some((&rest[1..close], target, end + 1))
}

/// `![alt](source)`: read and drop it, so no remote image is ever asked for.
fn image(rest: &str) -> Option<usize> {
    let close = rest[2..].find(']')? + 2;
    if rest[close + 1..].chars().next()? != '(' {
        return None;
    }
    let after = close + 2;
    let end = rest[after..].find(')')? + after;
    Some(end + 1)
}

/// `<https://...>` becomes a linked address; any other `<...>` is markup and is dropped.
fn angle(rest: &str) -> Option<(String, Option<String>, usize)> {
    if rest.starts_with("<!--") {
        let len = rest.find("-->")? + 3;
        return Some((String::new(), None, len));
    }
    let end = rest[1..].find('>')? + 1;
    let inner = &rest[1..end];
    let text = inner.trim();
    if let Some(url) = safe_link(text) {
        return Some((text.to_string(), Some(url), end + 1));
    }
    if text.contains(char::is_whitespace) || text.is_empty() {
        // A block of raw HTML, not an autolink.
        return Some((String::new(), None, end + 1));
    }
    if text.contains(':') || text.starts_with('/') {
        // A scheme Sigil refuses (http, mailto, javascript) or a closing tag: drop it.
        return Some((String::new(), None, end + 1));
    }
    // `<word>` is a tag such as `<b>`; remove it rather than show markup.
    if text.chars().all(|c| c.is_alphanumeric() || matches!(c, '/' | '-' | '_')) {
        return Some((String::new(), None, end + 1));
    }
    None
}

fn safe_link(url: &str) -> Option<String> {
    let url = url.trim();
    if url.is_empty() || url.len() > MAX_LINK {
        return None;
    }
    crate::mod_sources::check_url(url).ok()?;
    Some(url.to_string())
}

fn entity(rest: &str) -> Option<(String, usize)> {
    let end = rest[1..].find(';').map(|at| at + 1)?;
    if end > 12 {
        return None;
    }
    let body = &rest[1..end];
    let text = match body {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" | "#39" => "'",
        "nbsp" => " ",
        _ => {
            let number = body.strip_prefix("#x").or_else(|| body.strip_prefix("#X"));
            let value = if let Some(hex) = number {
                u32::from_str_radix(hex, 16).ok()
            } else if let Some(dec) = body.strip_prefix('#') {
                dec.parse::<u32>().ok()
            } else {
                None
            }?;
            let c = char::from_u32(value)?;
            if c.is_control() { " " } else { return Some((c.to_string(), end + 1)); }
        }
    };
    Some((text.to_string(), end + 1))
}

// --- raw HTML ----------------------------------------------------------------------------

fn strip_html_blocks(source: &str) -> String {
    let mut out = source.to_string();
    for tag in ["script", "style"] {
        out = remove_tagged(&out, tag);
    }
    while let Some(start) = out.find("<!--") {
        if let Some(end) = out[start + 4..].find("-->") {
            out.replace_range(start..start + 4 + end + 3, "");
        } else {
            out.truncate(start);
            break;
        }
    }
    out
}

fn remove_tagged(source: &str, tag: &str) -> String {
    let lower = source.to_ascii_lowercase();
    let open = format!("<{tag}");
    let close = format!("</{tag}");
    let mut out = String::new();
    let mut i = 0;
    while i < source.len() {
        let Some(at) = lower[i..].find(&open).map(|at| i + at) else {
            out.push_str(&source[i..]);
            break;
        };
        let after = lower[at + open.len()..].chars().next();
        if !after.is_some_and(|c| c == '>' || c == '/' || c.is_whitespace()) {
            out.push_str(&source[i..at + open.len()]);
            i = at + open.len();
            continue;
        }
        let Some(close_at) = lower[at..].find(&close).map(|offset| at + offset) else {
            out.push_str(&source[i..at]);
            break;
        };
        let Some(close_end) = lower[close_at..].find('>').map(|at| close_at + at) else {
            break;
        };
        out.push_str(&source[i..at]);
        i = close_end + 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DESCRIPTION: &str = include_str!("../tests/fixtures/mod-sources/description.md");
    const MALICIOUS: &str = include_str!("../tests/fixtures/mod-sources/malicious.md");

    fn all_links(source: &str) -> Vec<String> {
        parse(source).into_iter().flat_map(|b| b.links).map(|l| l.url).collect()
    }

    #[test]
    fn reads_headings_bullets_code_and_emphasis() {
        let blocks = parse(DESCRIPTION);
        assert_eq!(blocks[0].kind, BlockKind::Heading(2));
        assert_eq!(blocks[0].text, "Blue Haze");
        let paragraph = blocks.iter().find(|b| b.text.contains("bold")).unwrap();
        assert!(paragraph.spans.iter().any(|s| s.bold && &paragraph.text[s.start as usize..s.end as usize] == "bold"));
        assert!(paragraph.spans.iter().any(|s| s.italic));
        assert!(paragraph.spans.iter().any(|s| s.code));
        assert!(blocks.iter().any(|b| b.kind == BlockKind::Bullet && b.text.starts_with("Smaller")));
        assert!(blocks.iter().any(|b| b.kind == BlockKind::Quote));
        assert!(blocks.iter().any(|b| b.kind == BlockKind::Code && b.text.contains("raw *text*")));
        assert!(blocks.iter().any(|b| b.text.contains("a  ·  b")));
    }

    #[test]
    fn keeps_only_public_https_links() {
        assert_eq!(
            all_links(DESCRIPTION),
            ["https://mods.example.org/guide".to_string(), "https://mods.example.org/faq".to_string()]
        );
        let links = all_links(MALICIOUS);
        assert_eq!(links, ["https://mods.example.org/ok".to_string()]);
        assert!(links.iter().all(|u| crate::mod_sources::check_url(u).is_ok()));
    }

    #[test]
    fn drops_raw_html_and_remote_images() {
        let blocks = parse(MALICIOUS);
        let text = blocks.iter().map(|b| b.text.as_str()).collect::<Vec<_>>().join("\n");
        assert!(!text.contains("<script"), "{text}");
        assert!(!text.contains("<img"), "{text}");
        assert!(!text.contains("<a href"), "{text}");
        assert!(!text.contains("javascript:"), "{text}");
        assert!(!text.contains(".png"), "{text}");
        assert!(!text.contains("alert(\"raw\")"), "{text}");
        assert!(!text.contains("body{"), "{text}");
    }

    #[test]
    fn every_range_stays_inside_its_text_and_sorted() {
        for source in [DESCRIPTION, MALICIOUS] {
            for block in parse(source) {
                let len = block.text.len() as u32;
                let mut previous = 0;
                for span in &block.spans {
                    assert!(span.start < span.end && span.end <= len, "{span:?} in {block:?}");
                    assert!(span.start >= previous, "overlap in {block:?}");
                    previous = span.end;
                }
                for link in &block.links {
                    assert!(link.start < link.end && link.end <= len, "{link:?} in {block:?}");
                }
            }
        }
    }

    #[test]
    fn plain_text_projection_is_readable() {
        let text = to_text(DESCRIPTION);
        assert!(text.starts_with("Blue Haze\n\n"), "{text}");
        assert!(text.contains("• Smaller"), "{text}");
        assert!(!text.contains("**"), "{text}");
    }

    #[test]
    fn hostile_depth_and_size_do_not_grow_without_bound() {
        let deep = format!("{}x{}", "**".repeat(400), "**".repeat(400));
        let blocks = parse(&deep);
        assert!(blocks.len() <= MAX_BLOCKS);
        assert!(blocks.iter().all(|b| b.spans.len() <= MAX_SPANS));
        let long = "a".repeat(10_000);
        let blocks = parse(&long);
        assert_eq!(blocks.len(), 1);
    }
}
