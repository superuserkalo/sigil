# 0017 Opt-in product data ("Help build Sigil")

Status: accepted
Date: 2026-11-24, widened 2026-11-25

## Context

ADR 0014 made telemetry deliberately blind: allowlisted event names, no mod or hero, and no way
to link two events to one install. That tells us which pages are opened and nothing else. It
cannot feed recommendations, cannot tell which mods break after a game patch, which mods fight
each other, which configs help on which hardware, or what players look for and do not find.

The owner wants Sigil to collect product data broadly, as a long-term advantage no other
Deadlock mod manager has, on these terms (2026-11-25): it is off by default, the player is
asked during onboarding, and the player can turn it on or off at any time. This ADR widens the
first version, which covered recommendations only.

The limits below are what keeps this honest. They are part of the decision, not decoration.

## Decision

- One consent, "Help build Sigil", separate from crash reports and from the blind usage events.
  Off by default. Asked as its own step in onboarding (first run, and once for players who
  installed before this existed), with "Share" and "Not now" as equal buttons and a plain list
  of what is sent and what never is. A switch in Settings > Privacy turns it on or off at any
  time. Saying no changes nothing else in the app.
- It goes to Sigil's own Worker (`sigil-telemetry`). Never a third party, never an ad network,
  never sold or shared as raw data.
- What is sent, all keyed by a random install id stored only as a salted SHA-256:
  1. Mods: for GameBanana mods only, the mod id with installed, removed, turned on, turned off,
     updated, opened, saved, unsaved, "Not interested"; how long a mod stayed installed; which
     mods are enabled together; load order positions; variant chosen.
  2. Mod health: install result and failure reason code; overlap and conflict pairs by mod id;
     compatibility findings and repairs by mod id and game build; a mod turned off within a
     session of a game crash; the suspect mod of a crash card.
  3. Recommendations and Browse: which row and position a pick was shown in and whether it was
     opened or installed; filters and sort used; search text typed into Browse (trimmed to 64
     characters, lower case, dropped when it looks like an email, a path, a link or a long
     number) and how many results it had.
  4. Play: the most played heroes as a ranked list of at most ten hero ids with coarse playtime
     buckets; rank tier (not the exact rating); no match ids, no per-match data.
  5. Performance: the performance config applied (id and version), benchmark results (average
     and low frame rates, scene, resolution, settings preset), and hardware class: GPU model
     name, CPU model name, RAM rounded to 4 GB, display resolution and refresh rate.
  6. App use: which features are used and how often, settings chosen (theme, density, language,
     mature setting), launch counts for modded and vanilla, session length buckets, Sigil
     version, OS and OS build, game build.
- What is never sent: Steam id, account or persona name, friends, match ids or match history,
  chat, file names or paths, mods with no GameBanana id (local or private files), mod contents,
  API keys or tokens, IP address (not stored by the Worker), hardware serials or any id that
  outlives a reinstall, anything from the AI agent's conversations.
- The Worker stores no IP address or user agent. Raw events are kept 13 months, then deleted.
  Aggregates built from them (co-install counts, mod health per game build, frame rates per
  hardware class) are kept. Nothing is published or shown to other players for a mod pair, a
  hardware class or a search term until at least 10 different installs share it (counted as
  distinct contributors, see the amendment below).
- The player can see what was sent ("What Sigil shared" lists the last events locally) and can
  delete it: Settings > Privacy > "Delete my data" removes every raw event for their install
  id on the server and gives them a new id. Turning the switch off stops sending at once and
  clears the local queue.
- Every event type is an allowlisted, typed record on the app and on the Worker. There is no
  free-form payload. A field not in the schema is dropped, not stored.
- What comes back to the player for sharing: "players who use this also use", mod health
  warnings ("often breaks after a patch", "conflicts with a mod you have"), and frame rate
  expectations for their hardware class.
- `docs/privacy.md` lists every field before the first build that sends one, and the
  onboarding step links to it.

## Amendment 2026-11-25: counting distinct contributors

An install id is a random id the app makes, so one person can send many made-up ids. Aggregates
therefore count distinct contributors in 30-day epochs (`services/telemetry/src/aggregates.ts`):

- Network bucket. Each request gets a bucket: a keyed hash (HMAC-SHA256 under the Worker secret
  `PRODUCT_BUCKET_KEY`) of the epoch number and the caller's network prefix (an IPv4 /24 or an
  IPv6 /48), cut to 16 hex characters. It changes every epoch, so a network cannot be followed
  from one epoch to the next. The IP address and the prefix are never stored. Without the secret
  the Worker still stores raw records but writes and publishes no aggregate (fail closed).
- Open-epoch contributions (`product_contrib`) are the only place a bucket sits next to an install
  hash. They live only until their epoch closes (30 days plus 2 days of grace) and go at once with
  "Delete my data". A bucket admits at most 3 installs per epoch, counted from that table, so a
  deleted install frees its slot and nothing grows forever.
- Closing an epoch (daily cron) writes, per aggregate key, counts only to `product_totals`:
  eligible installs (seen on at least 2 days in the epoch, within the 3 per bucket), distinct
  buckets and distinct days. No install hash and no bucket. Then the epoch's contributions are
  deleted.
- A mod pair, a mod health finding, a hardware class or a search term is shown to players only
  when at least one closed epoch met the floor on its own: at least 10 eligible installs from at
  least 6 buckets on at least 3 days. Its strength is the sum over epochs that met the floor;
  other epochs add nothing. The open epoch is never shown. One network therefore can never
  qualify a key, in one epoch or by waiting.
- "Players who use this also use" ranks by a damped score (cosine similarity over that strength,
  times support / (support + 10)) and lists at most 20.
- "Delete my data" removes the install's raw events and its open-epoch contributions and gives a
  new id. Closed-epoch totals are anonymous counts with nothing that points to an install, so they
  stay; a deleted install may still be part of a count from an epoch that already closed.

A person who controls six or more networks for several days can still reach the floor. The rule
raises the cost of poisoning; it does not make it impossible.

## Consequences

- The privacy page's promise that no mod or hero name ever leaves the PC becomes "not unless
  you turn on Help build Sigil".
- Sigil holds data that describes one install's taste and hardware. The salted hash, the
  10-install floor, the 13-month limit, the local log and the delete button bound that.
- Search text is the riskiest field (free text). It is scrubbed on the device and again on the
  Worker, and can be dropped without touching the rest.
- An opt-in that is off by default will be on for a minority of players. The data is a sample,
  not a census; aggregates must say how many installs they rest on.
