#!/usr/bin/env bash
# Tout le dépôt, en conteneurs jetables : base, qualité et tests du backend, web app de bout en bout,
# serveur MCP (OAuth et outils contre le vrai backend).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
"$ROOT/db/test/run.sh"
(cd "$ROOT/backend" && cargo fmt --check && cargo clippy --all-targets -- -D warnings)
"$ROOT/backend/test.sh"
"$ROOT/web/test.sh"
"$ROOT/mcp/test.sh"
