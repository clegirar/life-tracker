#!/usr/bin/env bash
# Tout le dépôt, en conteneurs jetables : script de déploiement, base, qualité et tests du backend,
# web app de bout en bout, serveur MCP (OAuth et outils contre le vrai backend). En série ; la CI
# (.github/workflows/ci.yml) lance les mêmes scripts en parallèle.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
"$ROOT/deploy/test.sh"
"$ROOT/db/test/run.sh"
(cd "$ROOT/backend" && cargo fmt --check && cargo clippy --all-targets -- -D warnings)
"$ROOT/backend/test.sh"
"$ROOT/web/test.sh"
"$ROOT/mcp/test.sh"
