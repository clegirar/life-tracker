#!/usr/bin/env bash
# Le serveur MCP : types, build, tests OAuth, puis de bout en bout contre le vrai backend et un
# PostgreSQL 18.6 jetable. Le conteneur, le backend et la base disparaissent à la sortie.
# Usage : mcp/test.sh [arguments de node --test]   (Docker requis)
set -euo pipefail

MCP="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(dirname "$MCP")"
BACKEND="$ROOT/backend"
IMAGE="postgres:18.6"
C="lt-mcp-test-$$"
SERVER_PID=""
LOG="$(mktemp)"

cleanup() {
  [ -n "$SERVER_PID" ] && kill "$SERVER_PID" 2>/dev/null || true
  docker rm -f -v "$C" >/dev/null 2>&1 || true
  rm -f "$LOG"
}
trap cleanup EXIT

cd "$MCP"
[ -d node_modules ] || npm ci

echo "▶ Types et build"
npm run --silent typecheck
npm run --silent build

echo "▶ PostgreSQL jetable"
docker run -d --rm --name "$C" \
  -e POSTGRES_PASSWORD=test \
  -e TRACKER_OWNER_PASSWORD=owner-test \
  -e TRACKER_APP_PASSWORD=app-test \
  -e TRACKER_ADMIN_PASSWORD=admin-test \
  -v "$ROOT/db/init:/docker-entrypoint-initdb.d:ro" \
  -p 127.0.0.1:0:5432 \
  "$IMAGE" >/dev/null
for _ in $(seq 60); do
  if docker logs "$C" 2>&1 | grep -q 'PostgreSQL init process complete' \
     && docker exec "$C" pg_isready -q -U postgres; then
    ready=1
    break
  fi
  sleep 1
done
if [ -z "${ready:-}" ]; then
  docker logs "$C" >&2
  echo "ÉCHEC : le conteneur n'a pas démarré" >&2
  exit 1
fi
PORT="$(docker port "$C" 5432/tcp | head -1 | sed 's/.*://')"
url() { echo "postgres://$1:$2@127.0.0.1:$PORT/tracker"; }
free_port() { python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])'; }

echo "▶ Le vrai backend : migrate, sources, jeton de service, serve"
(cd "$BACKEND" && cargo build --quiet)
BIN="$BACKEND/target/debug/life-tracker"
admin() { ADMIN_DATABASE_URL="$(url tracker_admin admin-test)" "$BIN" admin "$@" 2>/dev/null; }
MIGRATE_DATABASE_URL="$(url tracker_owner owner-test)" "$BIN" migrate >/dev/null
LT_MCP_TEST_SERVICE_TOKEN="$(admin create-service-token mcp)"
LT_MCP_TEST_IOS_TOKEN="$(admin create-source ios)"
admin grant-kind ios coffee.cup
LT_MCP_TEST_LASTFM_TOKEN="$(admin create-source lastfm --silence-threshold "6 hours")"
admin grant-kind lastfm lastfm.scrobble
admin grant-kind lastfm lastfm.track_info
LT_MCP_TEST_GARMIN_TOKEN="$(admin create-source garmin --silence-threshold "12 hours")"
for kind in garmin.daily_summary garmin.sleep garmin.activity garmin.activity_track record.cancel; do
  admin grant-kind garmin "$kind"
done
LT_MCP_TEST_TWITCH_TOKEN="$(admin create-source twitch --silence-threshold "3 days" --label "Twitch")"
admin grant-kind twitch twitch.watch
LT_MCP_TEST_STEAM_TOKEN="$(admin create-source steam --silence-threshold "6 hours" --label "Steam")"
admin grant-kind steam steam.playtime
admin grant-kind steam steam.achievement
LT_MCP_TEST_IPHONE_TOKEN="$(admin create-source iphone --silence-threshold "2 days" --label "Position (iPhone)")"
admin grant-kind iphone overland.location
admin grant-kind iphone overland.visit
export LT_MCP_TEST_SERVICE_TOKEN LT_MCP_TEST_IOS_TOKEN LT_MCP_TEST_LASTFM_TOKEN LT_MCP_TEST_GARMIN_TOKEN LT_MCP_TEST_TWITCH_TOKEN LT_MCP_TEST_STEAM_TOKEN LT_MCP_TEST_IPHONE_TOKEN

API_PORT="$(free_port)"
# Une zone de confidentialité fictive, centrée sur le départ de la séance du jeu de données (et sur
# un lieu d'Overland).
PRIVACY_ZONES="43.212345,5.432109,600" DATABASE_URL="$(url tracker_app app-test)" LISTEN_ADDR="127.0.0.1:$API_PORT" "$BIN" serve >"$LOG" 2>&1 &
SERVER_PID=$!
for _ in $(seq 50); do
  curl -sf "http://127.0.0.1:$API_PORT/health" >/dev/null && up=1 && break
  sleep 0.2
done
[ -n "${up:-}" ] || { cat "$LOG" >&2; echo "ÉCHEC : le backend n'a pas démarré" >&2; exit 1; }
export LT_MCP_TEST_BACKEND_URL="http://127.0.0.1:$API_PORT"

echo "▶ Tests (OAuth, puis de bout en bout)"
OUT="$(mktemp)"
set +e
npx tsc -p tsconfig.json && node --test --test-concurrency=1 --test-reporter=spec "$@" 'build/test/*.test.js' | tee "$OUT"
status=${PIPESTATUS[0]}
set -e
none_skipped="$(grep -c 'skipped 0$' "$OUT" || true)"
rm -f "$OUT"
[ "$status" = 0 ] || exit "$status"
[ "$none_skipped" = 1 ] || { echo "ÉCHEC : des tests ont été sautés" >&2; exit 1; }

echo "▶ Aucun appel MCP n'a écrit : le journal ne contient que le jeu de données (4 cafés, 6 Last.fm, 4 Garmin, 5 Twitch, 3 Steam, 3 Overland)"
records="$(docker exec "$C" psql -U postgres -d tracker -Atc 'select count(*) from core.records')"
[ "$records" = 25 ] || { echo "ÉCHEC : $records enregistrements au journal" >&2; exit 1; }

echo "▶ Les logs du backend ne contiennent aucun jeton"
# Les heures des logs (« 21:27:55.432… ») contiennent parfois les chiffres des coordonnées
# cherchées (« 5.432 ») : on les retire avant de chercher.
if sed -E 's/[0-9]{2}:[0-9]{2}:[0-9]{2}(\.[0-9]+)?//g' "$LOG" | grep -qF -e "$LT_MCP_TEST_SERVICE_TOKEN" -e "$LT_MCP_TEST_IOS_TOKEN" -e "$LT_MCP_TEST_LASTFM_TOKEN" -e "$LT_MCP_TEST_GARMIN_TOKEN" -e "$LT_MCP_TEST_TWITCH_TOKEN" -e "$LT_MCP_TEST_STEAM_TOKEN" -e "$LT_MCP_TEST_IPHONE_TOKEN" -e "Jeu Fictif" -e "Succès fictif" -e "Veridis" -e "Ville Fictive" -e "43.21" -e "5.432"; then
  echo "ÉCHEC : secret ou donnée dans les logs" >&2
  exit 1
fi
