# Deadlock patch feeds

Status: active. Endpoint checked 2026-11-16.
Updated: 2026-11-16

The workbench needs to answer three questions after an update:

1. What did Valve say changed?
2. What changed in the game files?
3. Which paths in this project broke?

This note covers the first question. The game index and project baseline answer the other two.

## Official channel

Valve publishes Deadlock announcements through Steam Community announcements for app `1422450`.
The public Steam News API returns them without a key:

```text
https://api.steampowered.com/ISteamNews/GetNewsForApp/v2/?appid=1422450&count=30&maxlength=0
```

Response shape:

```json
{
  "appnews": {
    "appid": 1422450,
    "count": 228,
    "newsitems": [
      {
        "gid": "1848664643339845",
        "title": "Minor Update - 11-04-2026",
        "url": "https://steamstore-a.akamaihd.net/news/externalpost/steam_community_announcements/...",
        "author": "IceFrog",
        "contents": "[p]- Healbane: ...[/p]",
        "date": 1793854950,
        "feedname": "steam_community_announcements",
        "feed_type": 1,
        "tags": ["patchnotes"]
      }
    ]
  }
}
```

Useful fields:

- `gid`: stable id for deduplication and "new since last check".
- `title`: patch name, including the date.
- `date`: Unix seconds.
- `contents`: Steam BBCode, not HTML. Strip tags for display.
- `tags`: `patchnotes` marks balance and gameplay patches. Feature and event posts can still matter
  to mods, so keep them but label them.
- `url`: canonical announcement link.

The in-game News menu and the Steam store page show the same announcements. The official forums
carry a Changelog board at `https://forums.playdeadlock.com/forums/changelog.10/`, but automated
requests hit a browser challenge. Do not scrape it in the app. Link to it.

## What the app does with the feed

- Fetch on demand, not on a timer: when a page that shows patch notes opens with an empty cache
  (Stats fetches it itself then) or on refresh. No telemetry and no background heartbeat.
- Cache the last response under `<workspace>/.dah/patches.json`.
- Mark items whose `gid` was not in the previous cache as new.
- Show title, date, author and a plain-text excerpt. Link out for the full post.
- Correlate the newest patch date with the project baseline date. A baseline older than the latest
  patch is a prompt to run `dah mod baseline` after reviewing `dah mod check`.

The feed does not replace the file diff. Patch notes routinely omit path moves, material changes and
compiled-file changes. The index diff and project baseline are the ground truth.

## File-change channel

`GameTracking-Deadlock` tracks the installed game files per build. Use it for:

- A path that moved between builds.
- A file that was added or removed.
- A build id to name a baseline.

It does not replace a local index. A user can be on a different build, a beta branch or a modified
install.

## Network allowlist

Add exactly one host for this feature:

- `api.steampowered.com`

No analytics, no Steam login, no arbitrary news scraping. User-initiated fetch only.

## Sources

- [Steam News API, GetNewsForApp](https://steamcommunity.com/dev)
- [Deadlock Steam announcements](https://store.steampowered.com/news/app/1422450)
- [Official Deadlock changelog board](https://forums.playdeadlock.com/forums/changelog.10/)
- [GameTracking-Deadlock](https://github.com/SteamTracking/GameTracking-Deadlock)
- [SteamDB Deadlock patch notes](https://steamdb.info/app/1422450/patchnotes/)
