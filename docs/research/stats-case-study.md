# Stats tracker case study: Leetify, Blitz.gg and Scope.gg

Status: active. Researched 2026-11-16.
Updated: 2026-11-16

This note records what the best public match trackers show, why it works, and what the Deadlock
workbench should copy. It is a design input for `docs/research/stats.md` and the future Stats screen.

## Leetify

Leetify is a CS2 performance tracker built around impact, not the scoreboard.

What it shows:

- **Leetify Rating**: a win-impact metric that credits damage, trades, flash assists and other
  contributions, not only the final kill.
- **Aim Rating**: mechanical skill, benchmarked against the player base.
- **Utility Rating** and **Positioning Rating**.
- **KAST** (kill, assist, survive, trade), pre-aim, time to damage, counter-strafing.
- **Unused utility on death** and other mistake-oriented stats.
- Per-match reports with duel and round detail, highlights and friend comparison.
- Benchmark color coding: numbers are colored against recalculated population benchmarks, not shown
  as raw values with no context.

Why it works:

- It answers "why am I losing?" instead of listing everything the game already shows.
- It separates overall impact from mechanical aim. A player can be high impact and low aim, and the
  product explains that instead of collapsing everything into one score.
- Benchmarks and percentiles make a number meaningful.
- Match reports go deeper than the summary, so a bad result can be diagnosed.

What to copy for Deadlock:

- An impact rating based on damage, objectives, trades, healing/support and survival.
- Separate mechanics, positioning and objective/utility contributions when the API supports them.
- Benchmarks vs the player base and vs the player's own history.
- A short "what cost you this match" section, not just a scoreboard.
- Do not invent an aim rating for Deadlock unless the match data actually contains aim telemetry.

## Blitz.gg

Blitz is a multi-game companion with a Deadlock section.

Deadlock surfaces:

- Profile, match and rank tracking.
- Hero stats, tier list, counters and builds.
- In-game overlays: minimap health bars, ultimate timers, objective timers, item stats.
- Benchmarking: souls, damage, objective damage and buffs against players of the target rank as the
  match progresses.

Why it works:

- The profile answers "how am I doing over time" and the hero pages answer "what should I play and
  build next".
- Benchmarks are relative to rank, which makes in-match numbers actionable.
- The overlay is separate from the web profile. It does not try to be the same screen.
- Meta content (tier list, counters) sits next to personal stats, so the player can move from "my
  numbers" to "what to change".

What to copy for Deadlock:

- A player header with rank, recent form and trend.
- Hero pool performance, matchup/counter context and builds.
- Time-series comparison against rank benchmarks where data exists.
- Keep in-game overlays out of the first stats screen. They are a different product surface.

## Scope.gg

Scope is a CS2 analytics product focused on mistakes, maps and review.

What it shows:

- Performance summary and mistakes after each match.
- Map performance and position/heatmap guidance.
- Match scoreboard with contribution context.
- Utility usage maps and aim stats.
- Demo viewer, clips of best moments, player comparison.
- Prematch analytics and training tools.

Why it works:

- It turns stats into review. Maps and rounds are visual, not only tables.
- Heatmaps answer "where should I stand and where do I die?".
- Clips and demo review connect a number to the moment that caused it.
- Comparing players and prematch context give the numbers a purpose.

What to copy for Deadlock:

- Per-map and per-objective visualizations when the API provides positions or events.
- A match review surface, not just a list.
- "Where you lost the game" cards with map/objective context.
- Player comparison and recent-form context.
- If Deadlock data has no position stream, say so and use objective/area aggregates instead of a
  fake heatmap.

## What a good Deadlock tracker should show

1. **Player header**: rank, MMR or rating, matches, win rate, recent form, trend sparkline.
2. **Impact summary**: damage, objectives, healing/support, deaths, trades, KDA, contribution. Use
   an impact score with a clear formula and a benchmark.
3. **Hero pool**: games, win rate, KDA, damage and objective contribution per hero, sorted by
   sample size and confidence.
4. **Recent matches**: result, hero, duration, KDA, damage, objectives, rank change; expandable
   detail.
5. **Trends**: rank and win rate over time, rolling form, hero pick changes.
6. **Weaknesses**: the three metrics furthest below the player's rank benchmark, with a concrete
   next action.
7. **Maps and objectives**: heatmaps only if the data contains positions/events. Otherwise show
   objective control, lane/area aggregates and timing.
8. **Comparison**: versus friends, rank benchmark or the player's previous season.
9. **Review**: match timeline and notable moments when the data supports it.

## Visual language

- One strong summary row, then charts and lists. Not ten equal cards.
- Line chart for rank/win rate over time.
- Horizontal bars for hero pool and benchmark gaps.
- Percentile bands for impact, not raw numbers alone.
- Sparklines in the match list.
- Heatmaps only with real coordinate data, with a legend and sample count.
- Color for meaning: green above benchmark, amber neutral, red below. Never color-only; add labels.

## Data honesty rules

- Every derived metric has a formula and a source.
- Show sample size and confidence. A 1-game hero is not a 60 percent win rate story.
- No fabricated heatmaps, aim scores or positioning scores.
- If the public API lacks a field, the UI says what is unavailable and what would unlock it.
- Cache responses and respect rate limits.

## Sources

- https://leetify.com/
- https://leetify.com/blog/leetify-rating-explained/
- https://leetify.com/blog/leetify-stats-glossary/
- https://leetify.com/blog/cs2-benchmarks/
- https://blitz.gg/deadlock
- https://scope.gg/
- https://scope.gg/cs2-dashboard/
- https://scope.gg/headshot-stats/
