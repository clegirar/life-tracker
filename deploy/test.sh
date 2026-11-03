#!/usr/bin/env bash
# Tests de deploy/deploy.sh, sans Docker : un faux `docker` sur un PATH factice simule pull, up, ps et
# inspect, et un dossier temporaire tient lieu de dossier du projet sur le NAS. Puis shellcheck, s'il
# est installé.
# Usage : deploy/test.sh
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
SCRIPT="$HERE/deploy.sh"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# Le faux docker. Son comportement se règle par des fichiers de $FAKE : `fail_pull` (le tag dont le
# pull échoue), `state.<tag>` (une ligne par service : nom, état, santé, code de sortie, selon le tag
# en place dans .env). Chaque appel s'ajoute à `calls`, avec le tag lu dans .env à ce moment.
mkdir -p "$WORK/bin"
cat >"$WORK/bin/docker" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
tag="$(sed -n 's/^TRACKER_TAG=//p' .env 2>/dev/null || true)"
echo "$* [$tag]" >>"$FAKE/calls"
state="$FAKE/state.$tag"
[ -f "$state" ] || state="$FAKE/state.default"
case "$*" in
  "compose version") echo "Docker Compose version v2.20.1" ;;
  "compose config --services") printf '%s\n' db migrate backend web collector mcp ;;
  "compose pull -q") [ "$(cat "$FAKE/fail_pull" 2>/dev/null)" != "$tag" ] ;;
  "compose up -d") ;;
  "compose ps -a -q") awk '{print "id-" $1}' "$state" ;;
  inspect*) cat "$state" ;;
  *) echo "faux docker : appel inattendu : $*" >&2; exit 64 ;;
esac
EOF
chmod +x "$WORK/bin/docker"
FAKE_PATH="$WORK/bin:/usr/bin:/bin:/usr/sbin:/sbin"
# sha256sum vient de coreutils, ailleurs que dans /usr/bin sur un Mac.
SHA256SUM="$(command -v sha256sum)" || { echo "sha256sum requis" >&2; exit 1; }
ln -s "$SHA256SUM" "$WORK/bin/sha256sum"

HEALTHY='db running healthy 0
migrate exited none 0
backend running healthy 0
web running healthy 0
collector running starting 0
mcp running healthy 0'

failures=0
pass() { echo "  ok : $1"; }
ko() {
  echo "  ÉCHEC : $1" >&2
  failures=$((failures + 1))
}

# Un dossier de projet neuf : compose.yaml, .env avec l'ancien tag en 600, état sain par défaut.
setup() {
  rm -rf "$WORK/nas" "$WORK/fake"
  mkdir -p "$WORK/nas" "$WORK/fake"
  printf 'services: {}\n' >"$WORK/nas/compose.yaml"
  printf 'COMPOSE_PROJECT_NAME=life-tracker\nTRACKER_IMAGE_PREFIX=ghcr.io/owner/life-tracker\nTRACKER_TAG=v0.1.0\nMCP_PUBLIC_URL=https://nas.example.ts.net\n' >"$WORK/nas/.env"
  chmod 600 "$WORK/nas/.env"
  echo "$HEALTHY" >"$WORK/fake/state.default"
  SHA="$(sha256sum "$WORK/nas/compose.yaml" | cut -d' ' -f1)"
}

# run <SSH_ORIGINAL_COMMAND> [arguments] : code de sortie dans $rc, sortie dans $out.
run() {
  local request="$1"
  shift
  rc=0
  out="$(env -i HOME="$WORK" PATH="$FAKE_PATH" FAKE="$WORK/fake" \
    LT_DEPLOY_DIR="$WORK/nas" LT_DEPLOY_PATH="$FAKE_PATH" LT_DEPLOY_TIMEOUT="${TIMEOUT:-6}" \
    SSH_ORIGINAL_COMMAND="$request" bash "$SCRIPT" "$@" 2>&1)" || rc=$?
}

tag_is() { [ "$(sed -n 's/^TRACKER_TAG=//p' "$WORK/nas/.env")" = "$1" ]; }
no_docker() { [ ! -s "$WORK/fake/calls" ] || ! grep -q '^compose pull' "$WORK/fake/calls"; }

expect_refused() {
  local label="$1"
  shift
  setup
  run "$@"
  if [ "$rc" -eq 2 ] && tag_is v0.1.0 && no_docker; then pass "$label"; else ko "$label (code $rc) : $out"; fi
}

echo "▶ Commandes refusées"
setup
expect_refused "commande vide" ""
expect_refused "autre commande" "ls -la"
expect_refused "injection" "deploy v0.2.0 $SHA; rm -rf /"
expect_refused "version sans v" "deploy 0.2.0 $SHA"
expect_refused "version non semver" "deploy v0.2 $SHA"
expect_refused "préversion" "deploy v0.2.0-rc.1 $SHA"
expect_refused "latest" "deploy latest $SHA"
expect_refused "empreinte trop courte" "deploy v0.2.0 ${SHA:0:63}"
expect_refused "empreinte en majuscules" "deploy v0.2.0 $(echo "$SHA" | tr 'a-f' 'A-F')"
expect_refused "saut de ligne final" "deploy v0.2.0 $SHA
"
expect_refused "argument en plus" "deploy v0.2.0 $SHA" extra
expect_refused "empreinte différente" "deploy v0.2.0 $(printf 'autre\n' | sha256sum | cut -d' ' -f1)"

echo "▶ Réussite"
setup
run "deploy v0.2.0 $SHA"
if [ "$rc" -eq 0 ] && tag_is v0.2.0; then pass "nouveau tag écrit"; else ko "réussite (code $rc) : $out"; fi
if grep -q '^compose pull -q \[v0.2.0\]' "$WORK/fake/calls" && grep -q '^compose up -d \[v0.2.0\]' "$WORK/fake/calls"; then
  pass "pull et up avec le nouveau tag"
else
  ko "pull et up : $(cat "$WORK/fake/calls")"
fi
if grep -q '^MCP_PUBLIC_URL=https://nas.example.ts.net$' "$WORK/nas/.env" && [ "$(wc -l <"$WORK/nas/.env" | tr -d ' ')" = 4 ]; then
  pass "le reste de .env intact"
else
  ko "reste de .env : $(cat "$WORK/nas/.env")"
fi
if [ -z "$(find "$WORK/nas/.env" -perm 600)" ]; then ko "droits de .env perdus"; else pass "droits de .env gardés (600)"; fi
if [ "$(find "$WORK/nas" -name '.env.*' | wc -l | tr -d ' ')" = 0 ]; then pass "aucun fichier temporaire"; else ko "fichier temporaire laissé"; fi
if grep -q 'ghcr.io\|nas.example' <<<"$out"; then ko "la sortie montre le contenu de .env : $out"; else pass "sortie sans contenu de .env"; fi

echo "▶ Échecs, retour à l'ancien tag"
setup
printf '%s\n' "$HEALTHY" | sed 's/^backend running healthy/backend running unhealthy/' >"$WORK/fake/state.v0.2.0"
run "deploy v0.2.0 $SHA"
if [ "$rc" -eq 1 ] && tag_is v0.1.0 && grep -q '^compose up -d \[v0.1.0\]' "$WORK/fake/calls"; then
  pass "service unhealthy : ancien tag remis et relancé"
else
  ko "unhealthy (code $rc) : $out"
fi

setup
printf '%s\n' "$HEALTHY" | sed 's/^migrate exited none 0/migrate exited none 1/' >"$WORK/fake/state.v0.2.0"
run "deploy v0.2.0 $SHA"
if [ "$rc" -eq 1 ] && tag_is v0.1.0; then pass "migrate en erreur"; else ko "migrate (code $rc) : $out"; fi

setup
echo v0.2.0 >"$WORK/fake/fail_pull"
run "deploy v0.2.0 $SHA"
if [ "$rc" -eq 1 ] && tag_is v0.1.0 && grep -q '^compose up -d \[v0.1.0\]' "$WORK/fake/calls"; then
  pass "pull en échec"
else
  ko "pull (code $rc) : $out"
fi

setup
printf '%s\n' "$HEALTHY" | grep -v '^mcp ' >"$WORK/fake/state.v0.2.0"
TIMEOUT=2 run "deploy v0.2.0 $SHA"
if [ "$rc" -eq 1 ] && tag_is v0.1.0 && grep -q 'mcp : absent' <<<"$out"; then
  pass "service absent jusqu'à la fin de l'attente"
else
  ko "absent (code $rc) : $out"
fi

setup
printf '%s\n' "$HEALTHY" | sed 's/^web running healthy/web running starting/' >"$WORK/fake/state.v0.2.0"
TIMEOUT=2 run "deploy v0.2.0 $SHA"
if [ "$rc" -eq 1 ] && tag_is v0.1.0; then pass "web encore « starting » à la fin de l'attente"; else ko "starting (code $rc) : $out"; fi

setup
printf '%s\n' "$HEALTHY" | sed 's/^collector running starting/collector exited none 1/' >"$WORK/fake/state.v0.2.0"
run "deploy v0.2.0 $SHA"
if [ "$rc" -eq 1 ] && tag_is v0.1.0; then pass "collecteur arrêté"; else ko "collecteur (code $rc) : $out"; fi

if command -v shellcheck >/dev/null; then
  echo "▶ shellcheck"
  if shellcheck "$SCRIPT" "$HERE/test.sh"; then pass "shellcheck"; else ko "shellcheck"; fi
else
  echo "▶ shellcheck absent : contrôle sauté"
fi

if [ "$failures" -ne 0 ]; then
  echo "ÉCHEC : $failures test(s) de deploy.sh" >&2
  exit 1
fi
echo "✔ deploy.sh"
