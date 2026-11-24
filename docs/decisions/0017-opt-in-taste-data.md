# 0017 Opt-in taste data for recommendations

Status: accepted, not built yet
Date: 2026-11-24

## Context

Browse has a "For you" row (`rust/crates/desktop/src/screens/browse/foryou.rs`) that is worked
out on the player's PC from the heroes they play, the mods they installed and their favorites.
It is a content-based recommender: it can only say "this mod is for a hero you play". The
recommendations players value most ("players who use this also use that") need to know which
mods are used together, and no single PC knows that.

ADR 0014 made telemetry deliberately blind: allowlisted event names, no mod or hero, and no way
to link two events to one player. That cannot feed a recommender. Deadlock Mod Manager asks at
first start and, on a yes, sends which mods a player opens and installs with an id for the PC,
to Google Analytics.

The owner asked on 2026-11-24 for Sigil to collect taste data as a long-term advantage, and
approved changing the rule. The limits below are what keeps that honest: the player says yes
first, sees exactly what is sent, and can take it back.

## Decision

- A new, separate consent: "Help improve recommendations". Off by default. Asked once in its
  own dialog that lists what is sent and what never is, in plain words, with "Share" and "No
  thanks" as equal buttons. It has its own switch in Settings > Privacy. Saying no to it does
  not turn off anything else, and the local "For you" row works the same either way.
- It goes to Sigil's own Worker (`sigil-telemetry`), never a third party, never an ad network.
- What is sent, and nothing else:
  - A random install id (the one usage events already use), stored only as a salted SHA-256,
    so one player's events can be linked to each other and to nobody.
  - Taste events, each with a GameBanana mod id and the day: installed, removed, turned on,
    turned off, opened, saved to favorites, removed from favorites, marked "Not interested".
  - For recommendation rows: which row and position a pick was shown in and whether it was
    opened or installed, so the ranking can be measured.
  - The player's most played heroes as a ranked list of at most five hero ids. No match data,
    no stats, no account.
  - App version and operating system.
- What is never sent: Steam id, account or persona name, match history, file names or paths,
  mods with no GameBanana id (local or private files), mod contents, search text, IP address,
  hardware ids.
- Mature mods: sent like any other when the player's setting shows them; never when it hides
  them.
- The Worker stores no IP address or user agent. Raw events are deleted after 180 days. The
  aggregates built from them (how often two mods are used together) are kept, and a pair is
  only published once at least 10 different installs share it, so a pair can never point at
  one player.
- The player can delete what was collected: Settings > Privacy > "Delete my recommendation
  data" removes every raw event for their install id and gives them a new id. Turning the
  switch off stops sending at once.
- What comes back: `GET /v1/taste/related` returns, for a mod id, the mods most used with it.
  The app asks for the mods the player has (ids only, with consent) and blends the answer into
  "For you". Without consent the app never calls it.
- `docs/privacy.md` lists every field before the first build that sends one.

## Consequences

- The privacy page's promise that no mod or hero name ever leaves the PC becomes "not unless
  you turn on recommendations sharing".
- Sigil holds data that could, with enough installs, describe one player's taste. The salted
  hash, the 10-install floor, the 180-day limit and the delete button are what bound that.
- The recommender has a path from content-based to item-to-item collaborative filtering
  (co-install counts) and later to learned models, without changing what is collected.
