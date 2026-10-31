#!/usr/bin/env bash
# Exécuté une seule fois par l'image postgres, à la création du volume (docker-entrypoint-initdb.d),
# en superutilisateur. Crée les trois rôles et la base du tracker ; le schéma vient ensuite des migrations.
set -euo pipefail

: "${TRACKER_OWNER_PASSWORD:?TRACKER_OWNER_PASSWORD manquant}"
: "${TRACKER_APP_PASSWORD:?TRACKER_APP_PASSWORD manquant}"
: "${TRACKER_ADMIN_PASSWORD:?TRACKER_ADMIN_PASSWORD manquant}"

psql -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" --dbname postgres \
  -v owner_pw="$TRACKER_OWNER_PASSWORD" \
  -v app_pw="$TRACKER_APP_PASSWORD" \
  -v admin_pw="$TRACKER_ADMIN_PASSWORD" <<'SQL'
-- Propriétaire du schéma : sert uniquement aux migrations (service « migrate »).
CREATE ROLE tracker_owner LOGIN PASSWORD :'owner_pw';
-- Backend à l'exécution : ajoute au journal, ne le modifie jamais.
CREATE ROLE tracker_app LOGIN PASSWORD :'app_pw';
-- Outil d'administration local : registres, purge, reconstruction.
CREATE ROLE tracker_admin LOGIN PASSWORD :'admin_pw';

CREATE DATABASE tracker OWNER tracker_owner;
REVOKE ALL ON DATABASE tracker FROM PUBLIC;
GRANT CONNECT ON DATABASE tracker TO tracker_app, tracker_admin;
SQL
