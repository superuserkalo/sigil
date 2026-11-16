//! `dah browse`: read-only Deadlock mod listing from GameBanana.
//!
//! This command calls the core directly instead of going through `domain.rs`:
//! the browse path is a stateless fetch and the packaging seam stays small.

use serde_json::json;

use deadlock_core::gamebanana::{
    GAMEBANANA_GAME_ID, GAMEBANANA_MAX_PERPAGE, fetch_deadlock_mods_page, format_browse_mods,
};

use crate::error::{CliError, Result};
use crate::util::print_json;

pub struct BrowseArgs {
    pub page: usize,
    pub limit: usize,
    pub json: bool,
}

pub fn run(args: &BrowseArgs) -> Result<()> {
    if args.page == 0 {
        return Err(CliError::new(
            "invalid_page",
            "Page must be 1 or greater. GameBanana pages start at 1.",
        ));
    }

    if args.limit == 0 || args.limit > GAMEBANANA_MAX_PERPAGE {
        return Err(CliError::new(
            "invalid_limit",
            format!(
                "Limit must be from 1 to {GAMEBANANA_MAX_PERPAGE}. GameBanana returns at most \
                 {GAMEBANANA_MAX_PERPAGE} records per request, use --page to read further."
            ),
        ));
    }

    let listing = fetch_deadlock_mods_page(args.page, args.limit)
        .map_err(|error| CliError::new("gamebanana", error.to_string()))?;

    if args.json {
        print_json(&json!({
            "ok": true,
            "source": format!("gamebanana:{GAMEBANANA_GAME_ID}"),
            "page": listing.page,
            "perPage": listing.per_page,
            "pageCount": listing.page_count(),
            "total": listing.total,
            "complete": listing.complete,
            "returned": listing.mods.len(),
            "items": listing.mods,
        }));
        return Ok(());
    }

    println!("{}", format_browse_mods(&listing));
    println!();
    println!("Read only: no files were downloaded and no account is used.");

    Ok(())
}
