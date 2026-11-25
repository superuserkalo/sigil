## What changed

<!-- A short summary. Link the issue it closes, if any: "Closes #123". -->

## Why

<!-- The problem this solves, or the decision record it follows. -->

## How it was tested

<!-- Commands you ran and what you checked by hand, on which OS. Say if it was not run on Windows. -->

## Checklist

- [ ] `cargo test -p dah-desktop -p deadlock-core` passes (and `npm test` in `services/telemetry` if the Worker changed)
- [ ] `cargo fmt` is clean
- [ ] Behaviour changes have a test, and the affected doc is updated
- [ ] New UI strings have their German entry
- [ ] No em dashes, no game files, VPKs or secrets, no code copied from GPL projects
- [ ] Nothing new leaves the player's PC without an opt-in that is off by default
