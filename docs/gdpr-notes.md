# GDPR notes for "Help build Sigil"

Status: notes, not legal advice. Have a lawyer who knows GDPR review before the opted-in base is large.
Owner is in the EU (Austria); many players are too.

What applies:
- Product data keyed by an install id is personal data (pseudonymous), even with no name or Steam id.
- Legal basis: consent. It must be freely given, specific, informed and as easy to withdraw as to
  give. In the app: an onboarding step with equal buttons, off by default, a switch in Settings.
- Storage limitation: data is kept "until you delete it", which must be stated where consent is
  given. Stated in the onboarding step, Settings > Privacy and `docs/privacy.md`.
- Rights to cover: access (the local "What Sigil shared" log shows recent records; a full server
  export does not exist yet), erasure ("Delete my data"), withdrawal (the switch).
- Processor: Cloudflare (Workers, D1) stores the data. Needs Cloudflare's data processing addendum
  on the account and a note of where data is stored.

Open items:
1. A privacy policy page with a named controller, contact address and the points above.
2. A way to export everything held for an install id (right of access and portability).
3. Record of processing activities (one page: what, why, how long, who can see it).
4. Children: Deadlock players may be under 16; decide whether to ask age or state 16+ in the policy.
5. Check whether a data protection impact assessment is expected for this scale and kind of data.
6. A contact address for privacy requests, and a process to answer within one month.
7. Breach plan: who is told, and within 72 hours for the authority (Austrian DSB).
