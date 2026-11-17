# Settings: social and integrations

Status: Integrations category built (2026-11-17). The community gallery is design only: it needs a
service that does not exist.

## What the reference apps have

**Grimoire Social** (Grimoire, MIT; `vendor/grimoire/docs/social-architecture.md`). A Cloudflare
Workers service (`grimoire-social`: Hono router, D1 SQLite, KV sessions, rate limits) that Grimoire
signs in to with Steam OpenID. It stores users (SteamID64, display name, avatar), published
portable profiles (a gzipped recipe of GameBanana mod ids, no mod files), one like per user per
profile, and reports. The app gets a Discover tab (top, new, by hero), Publish, Like, Report,
Delete account. The session token lives in the OS keychain. It sits behind an experimental flag.

**Deadlock Mod Manager** (GPL-3.0, behaviour studied only). Its settings have no account UI. Its
integrations are: GameBanana remote install (pairing with GameBanana so "install" on the website
queues a download in the app), DeadlockForge one-click installs (a localhost listener), Discord
game presence (fed by the game's `-condebug` console log), match data sharing to deadlock-api
(Steam's cache, plus an optional background Steam login that asks the game coordinator for match
salts), and Cloud Backup of profiles to DMM's own API behind a DMM sign-in. Server browser,
curated launch options and community crosshairs come from DMM's own API.

Sigil does not talk to `grimoire-social` or to DMM's API as if it were their client. Both are
private services run for those apps.

## What Sigil builds without a backend

Settings > Integrations:

| Item | How it works | Leaves the machine |
| --- | --- | --- |
| Discord Rich Presence | Local Discord IPC, hand-written frames (`screens/settings/discord.rs`). Page, optional mod count, modded or vanilla while the game runs. | Nothing over the network. Discord shows it to friends. Off by default. |
| Steam account | Reads `<Steam>/config/loginusers.vdf` (`deadlock_core::steam_users`), lists the accounts, links one. Stats opens on it; "Steam profile" and "Statlocker" open in the browser. | Nothing. The browser opens the pages. |
| GameBanana account | Paste a profile link or member id; Sigil reads the public name from `apiv11/Member/<id>/ProfilePage` and stores id and name. "Open profile" opens the browser. | One anonymous GET on Link. |
| Profile share codes | Already built in Profiles; the row opens Profiles. | As Profiles documents. |
| Community gallery | Shown as "Not available" with this reason. | Nothing. |

Settings > Privacy holds match data sharing (Steam cache only, no Steam login). See
`docs/architecture.md`, Network allowlist.

## What needs a service, and the service it would need

### Community gallery (Grimoire Social parity)

Publish a profile, browse others, like, report, delete your account.

- **Service:** a small HTTPS API owned by the Sigil project, for example on Cloudflare Workers
  with D1, the same shape as `grimoire-social`: `POST /auth/steam/start`, `GET
  /auth/steam/callback` (Steam OpenID 2.0, verifying `openid.claimed_id` against
  `steamcommunity.com/openid/login`), `GET /profiles?sort=top|new&hero=`, `POST /profiles`,
  `DELETE /profiles/:id`, `POST /profiles/:id/like`, `POST /profiles/:id/report`, `DELETE /me`.
- **Data:** users (SteamID64, display name, avatar, created, banned), profiles (8-char id, owner,
  title, description, mature flag, mod count, primary hero, the share-code payload Profiles
  already produces, like count, created, updated, deleted), likes (profile, voter), reports.
- **Client:** the session token in the OS keychain through the Rust app (AGENTS.md: credentials
  never enter the UI transcript, logs or a commit), a Discover page that imports through the
  existing Profiles import path, Publish from Profiles, and an account block in Integrations.
- **Policy:** opt-in, off by default, a new allowlist entry and an ADR in `docs/decisions/`
  (new host, new stored identity). Moderation and abuse handling need an owner before launch.

### GameBanana remote install (DMM parity)

"Install with Sigil" on a GameBanana page queues the download in the app. DMM pairs through
GameBanana's remote-install feature. Sigil would need GameBanana to register Sigil as a remote
install client (or a documented public pairing API); there is none to call today. Until then,
Browse and the `sigil://` style handoff are the way in.

### DeadlockForge one-click installs (Grimoire and DMM parity)

A localhost HTTP listener that deadlockforge.net posts mod ids to. No backend, but it opens a
local port that a web page can reach, so it needs a security review of the listener (origin
check, per-request confirm, no file paths from the page) before it is built. Not built in this
round.

### Cloud backup of profiles (DMM parity)

Needs the same Sigil service and sign-in as the gallery, plus storage per user. Local backups
(Settings > Backups) and share codes cover the offline half.

### Match sync through a Steam login (DMM parity)

DMM signs in to Steam as the user in the background and asks the game coordinator for match
salts. That needs the user's Steam session token and a Steam client library; it carries account
risk and is not something Sigil does. Sigil shares only what Steam's cache already holds.
