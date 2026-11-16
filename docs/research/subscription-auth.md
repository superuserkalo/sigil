# Subscription sign-in (BYOS)

Status: active. Implemented 2026-11-16. The browser flow is not run on this machine.
Updated: 2026-11-16

Bring your own subscription means the user signs in with an account they already
pay for, and the app calls the same first-party endpoint that account's own CLI
uses. It is not a subscription cookie being copied out of a browser, and it is not
a shared or resold credential.

fx (`https://fx.sh`, `https://github.com/vercel-labs/fx`, Apache-2.0) already
does this for Codex and Grok. The flow below is read from its source, not guessed.

## Codex (ChatGPT Plus, Pro, Business, Enterprise, Edu)

Observed in `src/core/auth/chatgpt_oauth.zig` and `src/gateway/openai_codex.zig`:

| Item | Value |
| --- | --- |
| Client id | `app_EMoamEEZ73f0CkXaXp7hrann` |
| Authorize | `https://auth.openai.com/oauth/authorize` |
| Token | `https://auth.openai.com/oauth/token` |
| Revoke | `https://auth.openai.com/oauth/revoke` |
| Scopes | `openid profile email offline_access api.connectors.read api.connectors.invoke` |
| Callback | loopback, ports 1455 and 1457 |
| Extra param | `codex_cli_simplified_flow=true` |
| API endpoint | `https://chatgpt.com/backend-api/codex/responses` |
| Headers | `Authorization: Bearer <access>`, `chatgpt-account-id: <id>`, `OpenAI-Beta: responses=experimental` |

PKCE S256, `state`, a five minute browser timeout, a refresh token, and an
account id carried in the token claims. The request body is the OpenAI Responses
shape, so the existing OpenAI Responses adapter can drive it with a custom
endpoint and headers.

## Grok (SuperGrok or X Premium)

Observed in `src/core/auth/grok_oauth.zig` and `src/gateway/xai_grok.zig`:

| Item | Value |
| --- | --- |
| Client id | `b1a00492-073a-47ea-816f-4c329264a828` |
| Authorize | `https://auth.x.ai/oauth2/authorize` |
| Token | `https://auth.x.ai/oauth2/token` |
| Revoke | `https://auth.x.ai/oauth2/revoke` |
| Scopes | `openid profile email offline_access grok-cli:access api:access` |
| API endpoint | `https://cli-chat-proxy.grok.com/v1/responses` |
| Headers | `Authorization: Bearer <access>`, `x-grok-model-override: <model>` |

Also PKCE S256 with a loopback callback, and the Responses shape.

## Vercel AI Gateway

fx supports two paths: a Vercel account login (OAuth) or an AI Gateway API key.
The Gateway is an OpenAI-compatible endpoint, so the key path already fits the
existing `chat` adapter. Account login is a separate OAuth flow and is deferred.

## What we will not do

- No cookie export, session scraping or browser-session replay.
- No shared accounts, no resale, no bundled subscription token.
- No token in a transcript, log, journal, screenshot or commit.
- No silent fallback to a different account. If the saved credential is broken,
  the app says so.

Provider terms still apply. A provider can change or block a third-party client.
The app must report that failure instead of pretending the subscription works.

## Implemented

`deadlock-core::subscription` owns PKCE, the authorize URL, token exchange,
refresh, revocation, the account-id claim, the per-provider request headers and a
store at `<data>/auth/<provider>.json` with owner-only permissions.

- `dah auth login codex|grok` opens the browser and waits on the loopback.
- `dah auth status` and `dah auth logout` never print a token.
- `dah auth use <provider> --model <id>` writes `<data>/host.json`, which the
  desktop reads before it starts the sidecar.
- The host supports `DAH_AGENT_PROVIDER=codex|grok`, refreshes near expiry and
  configures the OpenAI Responses adapter with the provider endpoint and headers.
- Settings shows the account and expiry, starts the CLI sign-in, and switches the
  host to the subscription.

Model ids come from the subscription catalog. fx's current defaults are
`gpt-5.6-luna` for Codex and `grok-4.5` for Grok; the exact id the account exposes
is the one to use. The host sends the provider headers the fx client sends:
`originator`, `OpenAI-Beta`, `chatgpt-account-id` and `accept` for Codex;
`X-XAI-Token-Auth`, `x-grok-client-identifier` and `x-grok-model-override` for
Grok.

The OAuth browser round trip has not been exercised here because it needs a real
subscription and a browser. The token exchange, refresh, JWT claim, store and
header shape are covered by tests, and the host starts against a fixture token.

## Rust design

1. `deadlock-core::subscription`
   - Provider table with the values above.
   - PKCE S256, `state`, loopback callback, authorize URL.
   - Token exchange and refresh, form encoded.
   - Token set: access, refresh, expiry, account id, id token. Debug is redacted.
   - Store under the app data directory with owner-only permissions, or the OS
     keychain in the desktop app.
   - `account_id` from the id token claim used by the provider.
2. `dah auth subscription login codex|grok`, `status`, `logout`.
   - Login prints the URL when a browser cannot be opened, and accepts a pasted
     code as a fallback for a blocked loopback.
   - Logout removes the local login and asks the provider to revoke it.
3. Agent host
   - `DAH_AGENT_PROVIDER=codex|grok` reads the stored token, refreshes when
     needed, and configures the OpenAI Responses adapter with the provider
     endpoint and headers. No token on the command line.
4. Desktop
   - Settings shows the connected account, expiry and a sign-out action. The
     token lives in the OS keychain. The sidecar receives a short-lived access
     token through its environment, never a refresh token.

## Tests

- PKCE S256 against the RFC 7636 vector.
- Authorize URL contains client id, redirect, scope, challenge and state.
- Token exchange and refresh against a local HTTP server, including an error
  response.
- Token store roundtrip, file mode on Unix, and redacted Debug.
- Codex and Grok request headers against a local Responses endpoint.
- Logout revokes and deletes, and a failed revoke is reported without keeping a
  usable local token.

## Evidence

- `https://fx.sh/docs/getting-started/authentication`
- `https://github.com/vercel-labs/fx` (`src/core/auth`, `src/gateway`)
- The source files named above, fetched 2026-11-16.
