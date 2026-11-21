//! A `u.toggle` is focusable, so it takes the press at its spot even with no handler. One
//! without `on_press` is a dead switch: inside a clickable row it must be `u.toggle_mark`.

use std::path::Path;

fn sources(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("source folder").flatten() {
        let path = entry.path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// The method chain from `start`, up to the end of the expression it sits in.
fn chain(text: &str, start: usize) -> &str {
    let mut depth = 0i32;
    for (at, c) in text[start..].char_indices() {
        match c {
            '(' | '{' | '[' => depth += 1,
            ')' | '}' | ']' if depth == 0 => return &text[start..start + at],
            ')' | '}' | ']' => depth -= 1,
            ',' | ';' if depth == 0 => return &text[start..start + at],
            _ => {}
        }
    }
    &text[start..]
}

#[test]
fn every_toggle_has_a_press_and_a_name() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    sources(&root, &mut files);
    let mut dead = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file).expect("source");
        for (at, _) in text.match_indices(".toggle(") {
            if !text[..at].trim_end().ends_with('u') {
                continue;
            }
            let before = text[..at].trim_end();
            if before.len() > 1 && before[..before.len() - 1].ends_with(|c: char| c.is_alphanumeric() || c == '_' || c == '.') {
                continue;
            }
            let call = chain(&text, at);
            if !call.contains("on_press") || !call.contains(".label(") {
                let line = text[..at].matches('\n').count() + 1;
                dead.push(format!("{}:{line}", file.display()));
            }
        }
    }
    assert!(dead.is_empty(), "toggles with no press or no name: {dead:#?}");
}
