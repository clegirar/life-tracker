#!/usr/bin/env bash
# Signe l'extension par AMO, en canal non listé (rien n'est publié sur addons.mozilla.org), et dépose
# le .xpi signé dans extension/web-ext-artifacts/. Sur le Mac seulement, jamais en CI.
# Les clés AMO (about:developers → « Gérer les clés d'API ») se lisent dans l'environnement, depuis le
# gestionnaire de mots de passe, jamais dans le dépôt ni dans un fichier (voir docs/deploiement.md) :
#   read -rs WEB_EXT_API_KEY && read -rs WEB_EXT_API_SECRET && export WEB_EXT_API_KEY WEB_EXT_API_SECRET
#   extension/sign.sh
# Chaque signature exige une version nouvelle : augmenter "version" dans extension/package.json avant.
set -euo pipefail

: "${WEB_EXT_API_KEY:?WEB_EXT_API_KEY manquant (clé JWT d'AMO, dans l'environnement)}"
: "${WEB_EXT_API_SECRET:?WEB_EXT_API_SECRET manquant (secret JWT d'AMO, dans l'environnement)}"

EXT="$(cd "$(dirname "$0")" && pwd)"
cd "$EXT"
[ -d node_modules ] || npm ci

"$EXT/test.sh"
# web-ext lit WEB_EXT_API_KEY et WEB_EXT_API_SECRET lui-même : ils ne passent pas sur la ligne de
# commande, donc ni dans `ps` ni dans l'historique.
npx web-ext sign --source-dir dist --artifacts-dir web-ext-artifacts --channel=unlisted
ls -1 web-ext-artifacts/*.xpi
