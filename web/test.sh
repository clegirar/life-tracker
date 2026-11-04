#!/usr/bin/env bash
# Tout ce qui concerne la web app : lint, types, tests unitaires, build, puis Playwright contre le vrai
# backend et un PostgreSQL 18.6 jetable. Le conteneur, le serveur et la base disparaissent à la sortie.
# Les captures restent dans web/e2e-artifacts/ (ignoré par git).
# Usage : web/test.sh [arguments de playwright test]   (Docker requis)
set -euo pipefail

WEB="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(dirname "$WEB")"
BACKEND="$ROOT/backend"
IMAGE="postgres:18.6"
C="lt-web-test-$$"
SERVER_PID=""

cleanup() {
  [ -n "$SERVER_PID" ] && kill "$SERVER_PID" 2>/dev/null || true
  docker rm -f -v "$C" >/dev/null 2>&1 || true
}
trap cleanup EXIT

cd "$WEB"
[ -d node_modules ] || npm ci
npx playwright install chromium >/dev/null

echo "▶ Lint, types, tests unitaires (Paris et un autre fuseau d'appareil), build"
npm run --silent lint
npm run --silent typecheck
TZ=Europe/Paris npm test --silent
TZ=America/New_York npm test --silent
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

echo "▶ Le vrai backend : migrate, set-password, create-source, serve"
(cd "$BACKEND" && cargo build --quiet)
BIN="$BACKEND/target/debug/life-tracker"
ARTIFACTS="$WEB/e2e-artifacts"
mkdir -p "$ARTIFACTS"
LOG="$ARTIFACTS/backend.log"

export E2E_USER="moi"
# Mot de passe d'un seul passage : il ne vit que le temps du script.
E2E_PASSWORD="e2e-$(openssl rand -hex 16)"
export E2E_PASSWORD

MIGRATE_DATABASE_URL="$(url tracker_owner owner-test)" "$BIN" migrate >/dev/null
printf '%s\n' "$E2E_PASSWORD" | ADMIN_DATABASE_URL="$(url tracker_admin admin-test)" "$BIN" admin set-password "$E2E_USER" 2>/dev/null
E2E_IOS_TOKEN="$(ADMIN_DATABASE_URL="$(url tracker_admin admin-test)" "$BIN" admin create-source ios 2>/dev/null)"
export E2E_IOS_TOKEN
ADMIN_DATABASE_URL="$(url tracker_admin admin-test)" "$BIN" admin grant-kind ios coffee.cup 2>/dev/null
E2E_LASTFM_TOKEN="$(ADMIN_DATABASE_URL="$(url tracker_admin admin-test)" "$BIN" admin create-source lastfm --silence-threshold "6 hours" 2>/dev/null)"
export E2E_LASTFM_TOKEN
ADMIN_DATABASE_URL="$(url tracker_admin admin-test)" "$BIN" admin grant-kind lastfm lastfm.scrobble 2>/dev/null
ADMIN_DATABASE_URL="$(url tracker_admin admin-test)" "$BIN" admin grant-kind lastfm lastfm.track_info 2>/dev/null
E2E_APPLE_TOKEN="$(ADMIN_DATABASE_URL="$(url tracker_admin admin-test)" "$BIN" admin create-source apple 2>/dev/null)"
export E2E_APPLE_TOKEN
ADMIN_DATABASE_URL="$(url tracker_admin admin-test)" "$BIN" admin grant-kind apple apple.play_history_row 2>/dev/null
E2E_GARMIN_TOKEN="$(ADMIN_DATABASE_URL="$(url tracker_admin admin-test)" "$BIN" admin create-source garmin --silence-threshold "12 hours" 2>/dev/null)"
export E2E_GARMIN_TOKEN
for kind in garmin.daily_summary garmin.sleep garmin.activity garmin.activity_track record.cancel; do
  ADMIN_DATABASE_URL="$(url tracker_admin admin-test)" "$BIN" admin grant-kind garmin "$kind" 2>/dev/null
done

API_PORT="$(free_port)"
# Sans DEV_INSECURE_COOKIE : le cookie est Secure, comme en production.
env -u DEV_INSECURE_COOKIE DATABASE_URL="$(url tracker_app app-test)" LISTEN_ADDR="127.0.0.1:$API_PORT" \
  "$BIN" serve >"$LOG" 2>&1 &
SERVER_PID=$!
for _ in $(seq 50); do
  curl -sf "http://127.0.0.1:$API_PORT/health" >/dev/null && up=1 && break
  sleep 0.2
done
[ -n "${up:-}" ] || { cat "$LOG" >&2; echo "ÉCHEC : le backend n'a pas démarré" >&2; exit 1; }

export E2E_API_URL="http://127.0.0.1:$API_PORT"
# Le conteneur jetable : un test de complétude y simule une source muette, rien d'autre.
export E2E_DB_CONTAINER="$C"
export API_TARGET="$E2E_API_URL"
E2E_WEB_PORT="$(free_port)"
export E2E_WEB_PORT

echo "▶ Playwright"
npx playwright test "$@"

echo "▶ Les logs du backend ne contiennent ni mot de passe ni token"
if grep -qF -e "$E2E_PASSWORD" -e "$E2E_IOS_TOKEN" -e "$E2E_LASTFM_TOKEN" -e "$E2E_APPLE_TOKEN" -e "$E2E_GARMIN_TOKEN" -e "Ville Fictive" -e "48.85" -e "Artiste Fictif" -e "Titre fictif" "$LOG"; then
  echo "ÉCHEC : secret dans les logs" >&2
  exit 1
fi
grep -q 'login succeeded' "$LOG"

echo "Captures : $ARTIFACTS/screenshots"
