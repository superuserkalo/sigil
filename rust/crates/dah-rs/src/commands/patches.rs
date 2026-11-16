//! `dah patches`: cache official Steam patch posts and flag new ones.

use std::collections::BTreeSet;
use std::path::Path;

use chrono::Utc;
use serde_json::{json, Value};

use crate::domain::{self, PatchNote};
use crate::error::{CliError, Result};
use crate::paths::CliPaths;
use crate::util::{parse_rfc3339, print_json};

const CACHE_TTL_SECONDS: i64 = 6 * 60 * 60;
const MAX_FETCH: usize = 50;
const SOURCE: &str = "steam-news:1422450";

pub struct PatchArgs {
    pub limit: usize,
    pub refresh: bool,
    pub json: bool,
    pub full: bool,
}

pub fn run(paths: &CliPaths, args: &PatchArgs) -> Result<()> {
    let limit = args.limit.clamp(1, MAX_FETCH);
    let cache_path = domain::patch_note_cache_path(&paths.workspace_dir);
    let previous = domain::load_patch_note_cache(&cache_path)
        .map_err(|error| CliError::new("patch_cache", error.to_string()))?;

    let cache_is_fresh = !args.refresh
        && !previous.notes.is_empty()
        && is_fresh(&previous.fetched_at)
        && previous.notes.len() >= limit;

    let mut warning: Option<String> = None;
    let mut from_cache = false;
    let fetched_at;
    let mut notes: Vec<PatchNote>;
    let mut new_ids: BTreeSet<String> = BTreeSet::new();

    if cache_is_fresh {
        from_cache = true;
        fetched_at = previous.fetched_at;
        notes = previous.notes;
    } else {
        match domain::fetch_patch_notes(limit) {
            Ok(fetched) => {
                if let Some(newest_known) = previous
                    .notes
                    .first()
                    .and_then(|note| parse_rfc3339(&note.published_at))
                {
                    let known: BTreeSet<&str> =
                        previous.notes.iter().map(|note| note.id.as_str()).collect();

                    new_ids = fetched
                        .iter()
                        .filter(|note| !known.contains(note.id.as_str()))
                        .filter(|note| {
                            parse_rfc3339(&note.published_at)
                                .is_some_and(|date| date > newest_known)
                        })
                        .map(|note| note.id.clone())
                        .collect();
                }

                let cache = domain::cache_from_notes(fetched.clone());
                fetched_at = cache.fetched_at.clone();
                notes = fetched;

                domain::save_patch_note_cache(&cache_path, &cache).map_err(|error| {
                    CliError::new(
                        "patch_cache",
                        format!("Could not write {}: {error}", cache_path.display()),
                    )
                })?;
            }
            Err(error) => {
                if previous.notes.is_empty() {
                    return Err(CliError::new(
                        "patch_notes",
                        format!(
                            "Could not fetch patch notes and no cache exists at {}: {error}",
                            cache_path.display()
                        ),
                    ));
                }

                warning = Some(format!(
                    "Could not refresh patch notes: {error}. Showing the cached copy."
                ));
                from_cache = true;
                fetched_at = previous.fetched_at;
                notes = previous.notes;
            }
        }
    }

    notes.truncate(limit);

    if args.json {
        print_json(&render_json(
            &cache_path,
            &fetched_at,
            from_cache,
            &notes,
            &new_ids,
            warning.as_deref(),
            args.full,
        ));
        return Ok(());
    }

    if notes.is_empty() {
        println!("No patch posts were returned by Steam.");
    } else {
        println!("Patch notes: {} post(s), {} new", notes.len(), new_ids.len());

        for note in &notes {
            let marker = if new_ids.contains(&note.id) {
                "NEW "
            } else {
                "    "
            };
            let kind = if note.is_patch { " [patch]" } else { "" };
            println!(
                "  {marker}{}  {}{kind}",
                short_date(&note.published_at),
                note.title
            );
        }
    }

    let cache_state = if from_cache { "cached" } else { "refreshed" };
    println!(
        "Cache: {} ({cache_state}, fetched {fetched_at})",
        cache_path.display()
    );

    if let Some(warning) = &warning {
        println!("WARNING {warning}");
    }

    if args.full {
        match notes.first() {
            Some(newest) => {
                println!();
                println!(
                    "Newest note: {} ({})",
                    newest.title,
                    short_date(&newest.published_at)
                );
                let body = domain::strip_patch_note_html(&newest.contents);
                if body.is_empty() {
                    println!("(the post has no text)");
                } else {
                    println!("{body}");
                }
            }
            None => println!("No note text to print."),
        }
    }

    Ok(())
}

fn short_date(published_at: &str) -> String {
    published_at.get(0..10).unwrap_or(published_at).to_string()
}

fn render_json(
    cache_path: &Path,
    fetched_at: &str,
    from_cache: bool,
    notes: &[PatchNote],
    new_ids: &BTreeSet<String>,
    warning: Option<&str>,
    full: bool,
) -> Value {
    json!({
        "ok": true,
        "source": SOURCE,
        "path": cache_path.to_string_lossy(),
        "fetchedAt": fetched_at,
        "fromCache": from_cache,
        "newCount": new_ids.len(),
        "items": notes
            .iter()
            .map(|note| json!({
                "id": note.id,
                "title": note.title,
                "url": note.url,
                "author": note.author,
                "publishedAt": note.published_at,
                "tags": note.tags,
                "isPatch": note.is_patch,
                "isNew": new_ids.contains(&note.id),
            }))
            .collect::<Vec<_>>(),
        "full": if full {
            notes.first().map(|note| json!({
                "id": note.id,
                "title": note.title,
                "url": note.url,
                "publishedAt": note.published_at,
                "contents": domain::strip_patch_note_html(&note.contents),
            }))
        } else {
            None
        },
        "warning": warning,
    })
}

fn is_fresh(fetched_at: &str) -> bool {
    parse_rfc3339(fetched_at)
        .map(|date| {
            let age = Utc::now().signed_duration_since(date).num_seconds();
            (0..=CACHE_TTL_SECONDS).contains(&age)
        })
        .unwrap_or(false)
}

