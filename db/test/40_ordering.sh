#!/usr/bin/env bash
# Ordre de visibilité des numéros du journal sous écritures concurrentes.
# Usage : 40_ordering.sh <conteneur>. Laisse des enregistrements de la source « ordering » (le journal
# n'efface rien) : à lancer uniquement sur une base jetable.
set -euo pipefail

C="$1"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"; docker exec "$C" psql -X -q -U postgres -d tracker -c "ALTER TABLE core.records ENABLE TRIGGER records_append_lock" >/dev/null' EXIT

sql() { docker exec -i "$C" psql -X -q -t -A -v ON_ERROR_STOP=1 -d tracker "$@"; }
ok() { echo "NOTICE:  ok  $1"; }
fail() { echo "ÉCHEC : $1" >&2; exit 1; }
insert() {
  echo "INSERT INTO core.records (source_id, kind, schema_version, key, occurred_at, payload)
        SELECT id, 'coffee.cup', 1, '$1', now(), '{}' FROM core.sources WHERE name = 'ordering'
        RETURNING seq;"
}

sql -U postgres -c "SELECT t.make_source('ordering', '{coffee.cup}')
                    WHERE NOT EXISTS (SELECT 1 FROM core.sources WHERE name = 'ordering')" >/dev/null

# 1. Avec le verrou : B attend que A ait validé, et son numéro vient après.
run_id="$(date +%s)"
printf 'BEGIN;\n%s\nSELECT pg_sleep(3);\nCOMMIT;\n' "$(insert "a-$run_id")" | sql -U tracker_app >"$TMP/a" &
pid_a=$!
sleep 1
printf 'SELECT extract(epoch FROM clock_timestamp());\n%s\nSELECT extract(epoch FROM clock_timestamp());\n' \
  "$(insert "b-$run_id")" | sql -U tracker_app >"$TMP/b" &
pid_b=$!
sleep 0.5
waiting="$(sql -U postgres -c "SELECT count(*) FROM pg_locks WHERE locktype = 'advisory' AND NOT granted")"
wait "$pid_a" "$pid_b"

seq_a="$(grep -m1 -E '^[0-9]+$' "$TMP/a")"
t0="$(sed -n 1p "$TMP/b")"; seq_b="$(sed -n 2p "$TMP/b")"; t1="$(sed -n 3p "$TMP/b")"
[ "$waiting" -ge 1 ] || fail "aucune session en attente du verrou d'ajout"
ok "une seconde écriture attend le verrou d'ajout"
awk -v a="$t0" -v b="$t1" 'BEGIN { exit !(b - a >= 1.5) }' || fail "B n'a pas attendu A ($t0 → $t1)"
ok "B a attendu la validation de A"
[ "$seq_b" -gt "$seq_a" ] || fail "seq de B ($seq_b) pas après celui de A ($seq_a)"
ok "le numéro de B ($seq_b) suit celui de A ($seq_a)"

# 2. Contre-test sans le verrou : un lecteur voit le numéro de B alors que celui de A manque.
sql -U postgres -c "ALTER TABLE core.records DISABLE TRIGGER records_append_lock" >/dev/null
printf 'BEGIN;\n%s\nSELECT pg_sleep(3);\nCOMMIT;\n' "$(insert "c-$run_id")" | sql -U tracker_app >"$TMP/c" &
pid_c=$!
sleep 1
seq_d="$(insert "d-$run_id" | sql -U tracker_app)"
gap="$(sql -U postgres -c "SELECT (SELECT count(*) FROM core.records WHERE seq = $seq_d) = 1
                                  AND (SELECT count(*) FROM core.records WHERE seq = $seq_d - 1) = 0")"
wait "$pid_c"
sql -U postgres -c "ALTER TABLE core.records ENABLE TRIGGER records_append_lock" >/dev/null
seq_c="$(grep -m1 -E '^[0-9]+$' "$TMP/c")"
[ "$gap" = "t" ] && [ "$seq_c" -eq $((seq_d - 1)) ] || fail "le contre-test n'a pas montré de trou (gap=$gap, c=$seq_c, d=$seq_d)"
ok "sans le verrou, le numéro $seq_d est visible avant $seq_c : le risque est réel"
