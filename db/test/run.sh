#!/usr/bin/env bash
# Teste le schéma sur un PostgreSQL 18 jetable : rôles, migrations, tests, puis sauvegarde
# et restauration dans un second conteneur où les tests sont rejoués.
# Usage : db/test/run.sh   (Docker requis ; rien n'est laissé derrière)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
IMAGE="postgres:18.6"
C1="lt-schema-test-$$"
C2="lt-schema-restore-$$"
DUMP="$(mktemp)"
SQL_TESTS=(10_privileges 20_dedup 30_purge 50_anonymization 60_generations 70_days_dst)

cleanup() { docker rm -f "$C1" "$C2" >/dev/null 2>&1 || true; rm -f "$DUMP"; }
trap cleanup EXIT

start() {
  docker run -d --rm --name "$1" \
    -e POSTGRES_PASSWORD=test \
    -e TRACKER_OWNER_PASSWORD=owner-test \
    -e TRACKER_APP_PASSWORD=app-test \
    -e TRACKER_ADMIN_PASSWORD=admin-test \
    -v "$ROOT/db/init:/docker-entrypoint-initdb.d:ro" \
    -v "$ROOT:/work:ro" \
    "$IMAGE" >/dev/null
  for _ in $(seq 60); do
    if docker logs "$1" 2>&1 | grep -q 'PostgreSQL init process complete' \
       && docker exec "$1" pg_isready -q -U postgres; then
      return 0
    fi
    sleep 1
  done
  docker logs "$1" >&2
  echo "ÉCHEC : $1 n'a pas démarré" >&2
  exit 1
}

psql_in() { local c="$1"; shift; docker exec -i "$c" psql -X -q -v ON_ERROR_STOP=1 -d tracker "$@"; }

# Les résultats des requêtes sont jetés ; les « ok » et les échecs arrivent sur stderr.
run_sql_tests() {
  psql_in "$1" -U postgres -f /work/db/test/lib.sql >/dev/null
  for t in "${SQL_TESTS[@]}"; do
    echo "── $t"
    psql_in "$1" -U postgres -f "/work/db/test/$t.sql" 2>&1 >/dev/null | sed 's/^psql:[^ ]* //'
  done
}

echo "▶ Démarrage de $IMAGE"
start "$C1"

echo "▶ Migrations (une transaction par fichier, en tracker_owner)"
for f in "$ROOT"/backend/migrations/*.sql; do
  echo "── $(basename "$f")"
  psql_in "$C1" -U tracker_owner -1 -f "/work/backend/migrations/$(basename "$f")"
done

echo "▶ Tests"
run_sql_tests "$C1"
echo "── 40_ordering"
"$ROOT/db/test/40_ordering.sh" "$C1"

echo "▶ Sauvegarde puis restauration dans un conteneur neuf"
docker exec "$C1" pg_dump -U postgres -Fc -N t tracker >"$DUMP"
start "$C2"
docker exec -i "$C2" pg_restore -U postgres -d tracker --exit-on-error <"$DUMP"
for q in \
  "SELECT last_value FROM core.records_seq_seq" \
  "SELECT count(*) || '/' || coalesce(max(seq), 0) FROM core.records" \
  "SELECT string_agg(tgname::text || ':' || tgenabled::text, ',' ORDER BY tgname) FROM pg_trigger WHERE tgrelid IN ('core.records'::regclass, 'core.purges'::regclass) AND NOT tgisinternal"
do
  a="$(psql_in "$C1" -U postgres -t -A -c "$q")"
  b="$(psql_in "$C2" -U postgres -t -A -c "$q")"
  [ "$a" = "$b" ] || { echo "ÉCHEC : après restauration, « $q » donne $b au lieu de $a" >&2; exit 1; }
done
echo "NOTICE:  ok  séquence, journal et triggers identiques après restauration"
run_sql_tests "$C2"

echo "▶ Tous les tests sont passés"
