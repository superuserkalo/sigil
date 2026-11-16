#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

pnpm -r --if-present typecheck
node --test packages/*/test/*.test.ts
