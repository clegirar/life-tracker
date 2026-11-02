-- Jetons de service de l'API de lecture (étape 9, voir docs/architecture.md).
-- Le serveur MCP lit avec un tel jeton, au niveau « pseudonymes », et n'écrit jamais : le jeton n'ouvre
-- que les routes de lecture, et rien ne le relie au registre des sources ni à l'ingestion.
-- Comme les tokens de source, seul le SHA-256 est stocké ; l'outil d'administration le crée.

CREATE TABLE core.service_tokens (
  id          smallint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  name        text NOT NULL UNIQUE CHECK (name ~ '^[a-z][a-z0-9_-]*$'),  -- jamais renommé
  token_hash  bytea NOT NULL UNIQUE CHECK (octet_length(token_hash) = 32),  -- SHA-256 du jeton
  created_at  timestamptz NOT NULL DEFAULT now(),
  revoked_at  timestamptz
);

-- Le backend vérifie un jeton, sans jamais en créer ni en révoquer.
GRANT SELECT ON core.service_tokens TO tracker_app;
GRANT SELECT, INSERT, UPDATE ON core.service_tokens TO tracker_admin;
