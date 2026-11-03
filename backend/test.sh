#!/usr/bin/env bash
# Tests du backend sur un PostgreSQL 18 jetable : rôles (db/init), migrations par `migrate`, puis les tests,
# par cargo-nextest s'il est installé (tous les binaires de test en parallèle), sinon par cargo test.
# Le conteneur n'utilise aucun volume de développement et disparaît à la sortie.
# Usage : backend/test.sh [arguments]   (Docker requis ; arguments de cargo nextest run ou de cargo test)
#         backend/test.sh --prepare   régénère le cache .sqlx/ (Docker et sqlx-cli requis)
set -euo pipefail

BACKEND="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(dirname "$BACKEND")"
IMAGE="postgres:18.6"
C="lt-backend-test-$$"

cleanup() { docker rm -f -v "$C" >/dev/null 2>&1 || true; }
trap cleanup EXIT

docker run -d --rm --name "$C" \
  -e POSTGRES_PASSWORD=test \
  -e TRACKER_OWNER_PASSWORD=owner-test \
  -e TRACKER_APP_PASSWORD=app-test \
  -e TRACKER_ADMIN_PASSWORD=admin-test \
  -v "$ROOT/db/init:/docker-entrypoint-initdb.d:ro" \
  -p 127.0.0.1:0:5432 \
  "$IMAGE" -c max_connections=300 >/dev/null
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

# Noms LT_TEST_* : un .env de développement chargé dans le shell ne peut pas devenir la cible des tests.
export LT_TEST_APP_URL="$(url tracker_app app-test)"
export LT_TEST_ADMIN_URL="$(url tracker_admin admin-test)"
export LT_TEST_OWNER_URL="$(url tracker_owner owner-test)"
# Seulement pour créer une base par test d'interprétation : ils se partageraient sinon la génération active.
export LT_TEST_SUPER_URL="$(url postgres test)"
cd "$BACKEND"

if [ "${1:-}" = "--prepare" ]; then
  # Le propriétaire, car il faut décrire les requêtes de serve et celles d'admin avec un seul rôle.
  sqlx migrate run --source migrations -D "$LT_TEST_OWNER_URL"
  DATABASE_URL="$LT_TEST_OWNER_URL" cargo sqlx prepare
  exit 0
fi

echo "▶ Première application des migrations par sqlx"
MIGRATE_DATABASE_URL="$LT_TEST_OWNER_URL" cargo run --quiet -- migrate

if command -v cargo-sqlx >/dev/null; then
  echo "▶ Le cache .sqlx/ correspond aux requêtes"
  DATABASE_URL="$LT_TEST_OWNER_URL" cargo sqlx prepare --check
fi

# Le crate n'a aucun test de documentation, que nextest ne lancerait pas : en ajouter demanderait
# `cargo test --doc` ici.
if cargo nextest --version >/dev/null 2>&1; then
  echo "▶ cargo nextest run"
  cargo nextest run "$@"
else
  echo "▶ cargo test"
  cargo test "$@"
fi
