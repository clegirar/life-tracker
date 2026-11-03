#!/usr/bin/env bash
# Version suivante d'un commit de main, pour le job `version` de .github/workflows/release.yml.
# Usage : deploy/next-version.sh <sha> <fichier des notes>
#
# Il y a release si le contenu d'une image a changé depuis le dernier tag vX.Y.Z atteignable :
# backend/, web/ ou mcp/, hors tests (backend/tests/, web/e2e/, mcp/test/) et Markdown. Le type des
# commits ne décide que du numéro : une rupture (`type!:`, `type(scope)!:` ou `BREAKING CHANGE:`)
# donne une mineure sous 1.0.0, une majeure ensuite ; un `feat`, une mineure ; tout le reste, un
# correctif. Sans tag atteignable, on part de v0.0.0 et de tout l'historique.
#
# Affiche le tag sur la sortie standard, ou rien s'il n'y a pas de release (le fichier des notes est
# alors vide). Les notes, en Markdown : Fonctionnalités, Corrections, Performances, puis « Autres
# changements » (refactor, build, chore, sujets hors convention) ; docs, test et ci n'y figurent pas.
# Rien non plus si le commit est plus ancien que la dernière version publiée (run relancé).
# Bash 3.2 (macOS) : ni mapfile, ni tableaux associatifs, ni ${var,,}.
set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "usage : $0 <sha> <fichier des notes>" >&2
  exit 2
fi
SHA="$(git rev-parse --verify --quiet "$1^{commit}")" || {
  echo "commit inconnu : $1" >&2
  exit 2
}
NOTES="$2"
: >"$NOTES"

SEMVER='^v([0-9]+)\.([0-9]+)\.([0-9]+)$'

# La plus haute version publiée doit être atteignable depuis ce commit.
NEWEST="$(git tag --list 'v[0-9]*' --sort=-v:refname | grep -E "$SEMVER" | head -n 1 || true)"
if [ -n "$NEWEST" ] && ! git merge-base --is-ancestor "$NEWEST" "$SHA"; then
  echo "$SHA précède $NEWEST : rien à publier" >&2
  exit 0
fi

if LAST="$(git describe --tags --abbrev=0 --match 'v[0-9]*' "$SHA" 2>/dev/null)"; then
  [[ "$LAST" =~ $SEMVER ]] || {
    echo "dernier tag illisible : $LAST" >&2
    exit 1
  }
  RANGE="$LAST..$SHA"
  CHANGED="$(git diff --name-only "$LAST" "$SHA" -- backend web mcp)"
else
  LAST=v0.0.0
  RANGE="$SHA"
  CHANGED="$(git ls-tree -r --name-only "$SHA" -- backend web mcp)"
fi
[[ "$LAST" =~ $SEMVER ]]
MAJOR="${BASH_REMATCH[1]}"
MINOR="${BASH_REMATCH[2]}"
PATCH="${BASH_REMATCH[3]}"

IMAGE_CHANGES="$(printf '%s\n' "$CHANGED" | grep -Ev '^(backend/tests/|web/e2e/|mcp/test/)|\.md$' | grep -v '^$' || true)"
if [ -z "$IMAGE_CHANGES" ]; then
  echo "aucun changement des images depuis $LAST" >&2
  exit 0
fi

# 0 : correctif, 1 : mineure, 2 : majeure.
LEVEL=0
FEATS=""
FIXES=""
PERFS=""
OTHERS=""
NL='
'
TYPE_RE='^([A-Za-z]+)(\([^)]*\))?(!?):'

while IFS= read -r -d '' message; do
  subject="${message%%"$NL"*}"
  body=""
  [ "$subject" = "$message" ] || body="${message#*"$NL"}"
  type=""
  breaking=0
  if [[ "$subject" =~ $TYPE_RE ]]; then
    type="$(printf '%s' "${BASH_REMATCH[1]}" | tr '[:upper:]' '[:lower:]')"
    [ -z "${BASH_REMATCH[3]}" ] || breaking=1
  fi
  if printf '%s\n' "$body" | grep -Eq '^BREAKING[ -]CHANGE:'; then breaking=1; fi

  if [ "$breaking" -eq 1 ]; then
    if [ "$MAJOR" -eq 0 ]; then
      [ "$LEVEL" -ge 1 ] || LEVEL=1
    else
      LEVEL=2
    fi
  elif [ "$type" = feat ] && [ "$LEVEL" -lt 1 ]; then
    LEVEL=1
  fi

  line="- $subject$NL"
  case "$type" in
    feat) FEATS="$FEATS$line" ;;
    fix) FIXES="$FIXES$line" ;;
    perf) PERFS="$PERFS$line" ;;
    docs | test | ci) ;;
    *) OTHERS="$OTHERS$line" ;;
  esac
done < <(git log -z --reverse --format='%s%n%b' "$RANGE")

case "$LEVEL" in
  2) NEXT="v$((MAJOR + 1)).0.0" ;;
  1) NEXT="v$MAJOR.$((MINOR + 1)).0" ;;
  *) NEXT="v$MAJOR.$MINOR.$((PATCH + 1))" ;;
esac

section() {
  [ -z "$2" ] || printf '## %s\n\n%s\n' "$1" "$2"
}
{
  section "Fonctionnalités" "$FEATS"
  section "Corrections" "$FIXES"
  section "Performances" "$PERFS"
  section "Autres changements" "$OTHERS"
} >"$NOTES"
[ -s "$NOTES" ] || printf 'Changements internes, sans entrée notable.\n' >"$NOTES"

echo "$NEXT"
