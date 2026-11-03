#!/usr/bin/env bash
# Déploiement d'une version sur le NAS, appelé par le job `deploy` de .github/workflows/release.yml.
# Lancé en root par sudo, depuis la commande forcée de la clé SSH dédiée : la seule entrée est
# SSH_ORIGINAL_COMMAND, `deploy vX.Y.Z <sha256 de compose.yaml>`. Copié à la main dans le dossier du
# projet, propriétaire root, droits 700 : marche à suivre dans docs/deploiement.md.
#
# Réécrit TRACKER_TAG dans .env, puis `pull` et `up -d`, et attend que les services soient sains.
# En cas d'échec, revient à l'ancien tag et sort en erreur. Ne pousse jamais de fichier : si
# compose.yaml diffère de celui du dépôt, il refuse. N'affiche que des noms de services et des états.
#
# LT_DEPLOY_DIR, LT_DEPLOY_TIMEOUT et LT_DEPLOY_PATH ne servent qu'à deploy/test.sh : sudo efface
# l'environnement de l'appelant, hors SSH_ORIGINAL_COMMAND que sudoers laisse passer.
set -euo pipefail
umask 077

DIR="${LT_DEPLOY_DIR:-/volume1/docker/life-tracker}"
# Le PATH de sudo ne contient pas toujours /usr/local/bin, où DSM met docker.
PATH="${LT_DEPLOY_PATH:-/usr/local/bin:/usr/local/sbin:/usr/bin:/usr/sbin:/bin:/sbin}"
export PATH
TIMEOUT="${LT_DEPLOY_TIMEOUT:-180}"
POLL=3
# Services dont « starting » suffit : le premier contrôle du collecteur n'a lieu qu'au bout de 5 min,
# et la page du jour signale déjà son silence. Il doit tout de même tourner.
SLOW_HEALTH=" collector "
# Services à usage unique : une sortie à 0 est leur succès.
ONE_SHOT=" migrate "

fail() {
  echo "REFUS : $*" >&2
  exit 2
}

[ "$#" -eq 0 ] || fail "aucun argument attendu"
REQUEST="${SSH_ORIGINAL_COMMAND:-}"
if [[ ! "$REQUEST" =~ ^deploy\ (v[0-9]{1,6}\.[0-9]{1,6}\.[0-9]{1,6})\ ([0-9a-f]{64})$ ]]; then
  fail "commande attendue : deploy vX.Y.Z <sha256 de compose.yaml>"
fi
TAG="${BASH_REMATCH[1]}"
WANT="${BASH_REMATCH[2]}"

cd "$DIR"
if [ ! -f compose.yaml ] || [ ! -f .env ]; then fail "compose.yaml ou .env absent de $DIR"; fi

HAVE="$(sha256sum compose.yaml | cut -d' ' -f1)"
if [ "$HAVE" != "$WANT" ]; then
  fail "compose.yaml du NAS différent de celui de $TAG : le copier à la main (docs/deploiement.md), puis relancer"
fi

PREVIOUS="$(sed -n 's/^TRACKER_TAG=//p' .env | tail -n 1)"
[[ "$PREVIOUS" =~ ^[A-Za-z0-9._-]+$ ]] || fail "TRACKER_TAG illisible dans .env"

if docker compose version >/dev/null 2>&1 </dev/null; then
  compose() { docker compose "$@" </dev/null; }
else
  compose() { docker-compose "$@" </dev/null; }
fi

# Réécrit .env par un fichier voisin puis mv : jamais de .env à moitié écrit. cp -p garde les droits
# et, en root, le propriétaire.
set_tag() {
  local tmp
  tmp="$(mktemp "$DIR/.env.XXXXXX")"
  if cp -p .env "$tmp" && sed "s/^TRACKER_TAG=.*/TRACKER_TAG=$1/" .env >"$tmp"; then
    mv "$tmp" .env
  else
    rm -f "$tmp"
    return 1
  fi
}

# Un service par ligne : nom, état, santé (none sans contrôle), code de sortie.
states() {
  local ids
  ids="$(compose ps -a -q)" || return 1
  [ -n "$ids" ] || return 0
  # shellcheck disable=SC2086 # un identifiant de conteneur par mot
  docker inspect --format '{{index .Config.Labels "com.docker.compose.service"}} {{.State.Status}} {{if .State.Health}}{{.State.Health.Status}}{{else}}none{{end}} {{.State.ExitCode}}' $ids </dev/null
}

# Affiche l'état de chaque service. 0 : tous sains ; 1 : échec ; 2 : pas encore.
# Appelée derrière `||`, donc sans set -e : chaque erreur se traite à la main.
check() {
  local services lines service found name status health code verdict=0
  services="$(compose config --services)" || return 1
  [ -n "$services" ] || return 1
  lines="$(states)" || return 1
  for service in $services; do
    found=""
    while read -r name status health code; do
      if [ "$name" = "$service" ]; then
        found=1
        break
      fi
    done <<<"$lines"
    if [ -z "$found" ]; then
      echo "  $service : absent"
      [ "$verdict" -eq 1 ] || verdict=2
      continue
    fi
    echo "  $service : $status $health"
    case "$status:$health" in
      exited:*)
        if [[ "$ONE_SHOT" == *" $service "* ]] && [ "$code" = 0 ]; then continue; fi
        verdict=1 ;;
      running:healthy | running:none) ;;
      running:unhealthy | dead:*) verdict=1 ;;
      running:starting)
        if [[ "$SLOW_HEALTH" == *" $service "* ]]; then continue; fi
        [ "$verdict" -eq 1 ] || verdict=2 ;;
      *) [ "$verdict" -eq 1 ] || verdict=2 ;;
    esac
  done
  return "$verdict"
}

# Attend au plus TIMEOUT secondes ; un échec net (unhealthy, sortie en erreur) n'attend pas.
wait_healthy() {
  local deadline=$((SECONDS + TIMEOUT)) final=0 out rc
  while :; do
    [ "$SECONDS" -lt "$deadline" ] || final=1
    rc=0
    out="$(check)" || rc=$?
    if [ "$rc" -ne 2 ] || [ "$final" = 1 ]; then
      echo "$out"
      [ "$rc" -eq 0 ]
      return
    fi
    sleep "$POLL"
  done
}

echo "▶ $PREVIOUS → $TAG"
set_tag "$TAG"
if compose pull -q && compose up -d && wait_healthy; then
  echo "✔ $TAG déployé"
  exit 0
fi

echo "✘ échec de $TAG : retour à $PREVIOUS" >&2
set_tag "$PREVIOUS"
if compose up -d && wait_healthy; then
  echo "retour à $PREVIOUS fait" >&2
else
  echo "retour à $PREVIOUS lui aussi en échec : intervenir à la main" >&2
fi
exit 1
