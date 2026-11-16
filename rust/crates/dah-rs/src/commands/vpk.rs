//! `dah vpk inspect`: read a packed directory VPK and verify its entries.

use std::collections::BTreeSet;
use std::path::PathBuf;

use serde_json::json;

use crate::error::{CliError, Result};
use crate::util::print_json;
use deadlock_core::vpk::{crc32, read_vpk_directory, read_vpk_entry_data};

pub struct InspectArgs {
    pub path: PathBuf,
    pub limit: usize,
    pub verify: bool,
    pub json: bool,
}

pub fn inspect(args: &InspectArgs) -> Result<bool> {
    let directory = read_vpk_directory(&args.path)
        .map_err(|error| CliError::new("vpk", error.to_string()))?;

    let mut archives = BTreeSet::new();
    let mut failures = Vec::new();
    let mut checked = 0usize;

    for entry in &directory.entries {
        archives.insert(entry.archive_index);

        if args.verify {
            checked += 1;
            let data = read_vpk_entry_data(&directory, entry)
                .map_err(|error| CliError::new("vpk", error.to_string()))?;
            let actual = crc32(&data);

            if actual != entry.crc32 {
                failures.push(json!({
                    "path": entry.path,
                    "expected": entry.crc32,
                    "actual": actual,
                }));
            }
        }
    }

    let shown: Vec<_> = directory.entries.iter().take(args.limit).collect();
    let ok = failures.is_empty();

    if args.json {
        print_json(&json!({
            "ok": ok,
            "path": args.path,
            "version": directory.header.version,
            "entries": directory.entries.len(),
            "shown": shown.len(),
            "skipped": directory.skipped,
            "duplicates": directory.duplicates,
            "archives": archives,
            "checked": checked,
            "failures": failures,
            "files": shown.iter().map(|entry| json!({
                "path": entry.path,
                "bytes": entry.total_length,
                "archive": entry.archive_index,
                "crc32": entry.crc32,
            })).collect::<Vec<_>>(),
        }));
        return Ok(ok);
    }

    println!(
        "VPK: {} (version {})",
        args.path.display(),
        directory.header.version
    );
    println!(
        "Entries: {} (showing {}), skipped {}, duplicates {}",
        directory.entries.len(),
        shown.len(),
        directory.skipped,
        directory.duplicates
    );
    println!(
        "Archives: {}",
        archives
            .iter()
            .map(|archive| if *archive == 0x7fff {
                "dir".to_string()
            } else {
                format!("{archive:03}")
            })
            .collect::<Vec<_>>()
            .join(", ")
    );

    for entry in shown {
        println!(
            "  {} ({} B, archive {})",
            entry.path,
            entry.total_length,
            if entry.archive_index == 0x7fff {
                "dir".to_string()
            } else {
                format!("{:03}", entry.archive_index)
            }
        );
    }

    if directory.entries.len() > args.limit {
        println!("  ... {} more", directory.entries.len() - args.limit);
    }

    if args.verify {
        if failures.is_empty() {
            println!("CRC32: {checked} entries verified.");
        } else {
            println!("CRC32: {} of {checked} entries failed.", failures.len());
            for failure in &failures {
                println!("  FAIL {failure}");
            }
        }
    } else {
        println!("CRC32: not checked. Pass --verify to read every entry.");
    }

    Ok(ok)
}
