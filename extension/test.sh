#!/usr/bin/env bash
# L'extension Firefox pour Twitch : types, tests unitaires (page, segmenteur, file avec un faux
# `fetch`, heure, lot validé contre contract/ingestion-v1.schema.json), build, puis `web-ext lint`.
# Ni Docker ni réseau, hors `npm ci` au premier passage.
# Usage : extension/test.sh [arguments de node --test]
set -euo pipefail

EXT="$(cd "$(dirname "$0")" && pwd)"
cd "$EXT"
[ -d node_modules ] || npm ci

echo "▶ Types"
npm run --silent typecheck

echo "▶ Tests unitaires"
rm -rf build
npx tsc -p tsconfig.json
# Le fuseau de Paris, pour vérifier le décalage local de l'heure d'été et d'hiver.
TZ=Europe/Paris node --test --test-reporter=spec "$@" 'build/test/*.test.js'

echo "▶ Build"
npm run --silent build

echo "▶ web-ext lint"
npx web-ext lint --source-dir dist --warnings-as-errors --output text
