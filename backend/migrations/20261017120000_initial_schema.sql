-- Schéma initial du POC (voir docs/database.md).
-- Appliquée par tracker_owner ; les rôles tracker_app et tracker_admin existent déjà (db/init/01-roles.sh).
-- Pas de migration descendante : le journal ne revient jamais en arrière.

CREATE EXTENSION IF NOT EXISTS pg_trgm;     -- recherche approximative (outil Rechercher)
CREATE EXTENSION IF NOT EXISTS btree_gist;  -- WITHOUT OVERLAPS sur une colonne text

-- core : la vérité (journal) et les référentiels ; migré avec soin.
-- derived : tout ce qui se recalcule depuis le journal ; jeté et reconstruit.
CREATE SCHEMA core;
CREATE SCHEMA derived;

CREATE TYPE core.metric_shape AS ENUM
  ('counter', 'rich_event', 'duration', 'state', 'measure', 'declaration', 'reference');
CREATE TYPE core.daily_aggregation AS ENUM ('count', 'sum', 'avg', 'max', 'last', 'none');
CREATE TYPE core.source_auth AS ENUM ('token', 'session');

-- ─── Référentiels ────────────────────────────────────────────────────────────

CREATE TABLE core.settings (
  id            boolean PRIMARY KEY DEFAULT true CHECK (id),  -- une seule ligne
  launch_date   date,                                         -- NULL tant que l'app n'est pas en ligne
  timezone      text NOT NULL DEFAULT 'Europe/Paris'          -- découpage des journées, à la lecture
);

CREATE TABLE core.kinds (
  kind                    text PRIMARY KEY CHECK (kind ~ '^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*$'),
  current_schema_version  smallint NOT NULL CHECK (current_schema_version > 0),
  stateful                boolean NOT NULL DEFAULT false,     -- interprète avec mémoire
  description             text NOT NULL
);

CREATE TABLE core.sources (
  id                 smallint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  name               text NOT NULL UNIQUE CHECK (name ~ '^[a-z][a-z0-9_-]*$'),  -- jamais renommé
  auth               core.source_auth NOT NULL,
  token_hash         bytea UNIQUE CHECK (octet_length(token_hash) = 32),       -- SHA-256 du token
  silence_threshold  interval,                                                 -- NULL : jamais muette
  last_seen_at       timestamptz,
  client_version     text,
  rejected_count     bigint NOT NULL DEFAULT 0,
  last_rejected_at   timestamptz,
  created_at         timestamptz NOT NULL DEFAULT now(),
  revoked_at         timestamptz,
  CHECK ((auth = 'token') = (token_hash IS NOT NULL))
);

-- Types qu'une source a le droit d'envoyer. On ne supprime jamais une ligne : on la révoque.
CREATE TABLE core.source_kinds (
  source_id   smallint NOT NULL REFERENCES core.sources,
  kind        text NOT NULL REFERENCES core.kinds,
  granted_at  timestamptz NOT NULL DEFAULT now(),
  revoked_at  timestamptz,
  PRIMARY KEY (source_id, kind)
);

CREATE TABLE core.metrics (
  metric               text PRIMARY KEY,
  label                text NOT NULL,
  shape                core.metric_shape NOT NULL,
  unit                 text,
  domain               text NOT NULL,
  daily_aggregation    core.daily_aggregation NOT NULL,
  item_levels          text[] NOT NULL DEFAULT '{}',  -- ex. {artist,album,track}, {contact}
  discovery            boolean NOT NULL DEFAULT false,
  -- Une déclaration ou une référence éclaire une autre métrique ; elle n'est jamais affichée seule.
  applies_to           text REFERENCES core.metrics DEFERRABLE INITIALLY DEFERRED,
  -- La référence qui fournit la durée d'un élément (durée d'un titre pour les écoutes).
  duration_from        text REFERENCES core.metrics DEFERRABLE INITIALLY DEFERRED,
  freshness_threshold  interval,                       -- alerte douce : aucune donnée depuis…
  description          text,
  CHECK ((shape IN ('declaration', 'reference')) = (applies_to IS NOT NULL)),
  CHECK (NOT discovery OR cardinality(item_levels) > 0)
);

-- Réconciliation « source prioritaire par période » : une seule source fait foi
-- pour une métrique sur un intervalle donné.
CREATE TABLE core.source_precedence (
  metric        text NOT NULL REFERENCES core.metrics,
  source_id     smallint NOT NULL REFERENCES core.sources,
  valid_during  tstzrange NOT NULL CHECK (NOT isempty(valid_during)),
  PRIMARY KEY (metric, valid_during WITHOUT OVERLAPS)
);

-- Annuaire : supprimer une personne supprime ses pseudonymes, ce qui anonymise son historique.
CREATE TABLE core.people (
  id            uuid PRIMARY KEY DEFAULT uuidv7(),
  display_name  text NOT NULL,
  created_at    timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE core.pseudonyms (
  pseudonym   text PRIMARY KEY,
  person_id   uuid NOT NULL REFERENCES core.people ON DELETE CASCADE,
  created_at  timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX ON core.pseudonyms (person_id);

-- Compte unique de la web app et ses sessions.
CREATE TABLE core.account (
  id             boolean PRIMARY KEY DEFAULT true CHECK (id),
  username       text NOT NULL,
  password_hash  text NOT NULL CHECK (password_hash LIKE '$argon2id$%'),
  updated_at     timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE core.sessions (
  token_hash    bytea PRIMARY KEY CHECK (octet_length(token_hash) = 32),  -- SHA-256 du cookie
  created_at    timestamptz NOT NULL DEFAULT now(),
  last_seen_at  timestamptz NOT NULL DEFAULT now(),
  expires_at    timestamptz NOT NULL
);
CREATE INDEX ON core.sessions (expires_at);

-- ─── Journal brut ────────────────────────────────────────────────────────────

-- Registre des purges : ajout seul, jamais le contenu purgé.
CREATE TABLE core.purges (
  id            bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  executed_at   timestamptz NOT NULL DEFAULT now(),
  executed_by   text NOT NULL DEFAULT current_user,
  selector      jsonb NOT NULL,
  reason        text NOT NULL,
  record_count  integer NOT NULL CHECK (record_count >= 0)
);

CREATE TABLE core.records (
  -- CACHE 1 + verrou d'ajout : les numéros deviennent visibles dans l'ordre.
  seq                 bigint GENERATED ALWAYS AS IDENTITY (CACHE 1) PRIMARY KEY,
  source_id           smallint NOT NULL,
  kind                text NOT NULL,
  schema_version      smallint NOT NULL CHECK (schema_version > 0),
  key                 text NOT NULL CHECK (length(key) BETWEEN 1 AND 512),
  occurred_at         timestamptz NOT NULL,
  utc_offset_minutes  smallint CHECK (utc_offset_minutes BETWEEN -900 AND 900),  -- NULL : inconnu (« Z »)
  received_at         timestamptz NOT NULL DEFAULT now(),
  client_version      text,
  request_id          uuid,
  payload             jsonb,                         -- NULL uniquement après une purge
  purge_id            bigint REFERENCES core.purges,
  UNIQUE (source_id, key),
  -- Le type doit être déclaré pour la source : vérifié par la base, pas seulement par l'API.
  FOREIGN KEY (source_id, kind) REFERENCES core.source_kinds,
  CHECK ((payload IS NULL) = (purge_id IS NOT NULL))
);
CREATE INDEX ON core.records (kind, seq);           -- avancée des interprètes
CREATE INDEX ON core.records (source_id, seq);      -- rejeu source par source
CREATE INDEX ON core.records (source_id, occurred_at);  -- sélecteurs de purge

-- Un seul ajout à la fois : sans ce verrou, une transaction pourrait rendre visible
-- le numéro 11 avant le 10, et un interprète qui avance son curseur sauterait le 10.
CREATE FUNCTION core.records_append_lock() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  PERFORM pg_advisory_xact_lock(7461001);
  RETURN NULL;
END
$$;
CREATE TRIGGER records_append_lock BEFORE INSERT ON core.records
  FOR EACH STATEMENT EXECUTE FUNCTION core.records_append_lock();

-- Ajout seul. Seule exception : la rédaction d'une purge, qui vide la charge utile
-- et pose purge_id, une seule fois, sans rien toucher d'autre.
CREATE FUNCTION core.records_guard() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF TG_OP = 'UPDATE' THEN
    IF OLD.purge_id IS NULL AND NEW.purge_id IS NOT NULL AND NEW.payload IS NULL
       AND (to_jsonb(NEW) - 'payload' - 'purge_id') = (to_jsonb(OLD) - 'payload' - 'purge_id') THEN
      RETURN NEW;
    END IF;
  END IF;
  RAISE EXCEPTION 'core.records est en ajout seul (% refusé)', TG_OP USING ERRCODE = 'LT001';
END
$$;
CREATE TRIGGER records_guard_row BEFORE UPDATE OR DELETE ON core.records
  FOR EACH ROW EXECUTE FUNCTION core.records_guard();
CREATE TRIGGER records_guard_truncate BEFORE TRUNCATE ON core.records
  FOR EACH STATEMENT EXECUTE FUNCTION core.records_guard();

CREATE FUNCTION core.purges_guard() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  RAISE EXCEPTION 'core.purges est en ajout seul (% refusé)', TG_OP USING ERRCODE = 'LT002';
END
$$;
CREATE TRIGGER purges_guard_row BEFORE UPDATE OR DELETE ON core.purges
  FOR EACH ROW EXECUTE FUNCTION core.purges_guard();
CREATE TRIGGER purges_guard_truncate BEFORE TRUNCATE ON core.purges
  FOR EACH STATEMENT EXECUTE FUNCTION core.purges_guard();

-- ─── Dérivé (recalculable) ───────────────────────────────────────────────────

-- Une génération = une interprétation complète du journal. On construit la suivante
-- à côté, puis on bascule en changeant une ligne.
CREATE TYPE derived.generation_state AS ENUM ('building', 'active', 'retired');

CREATE TABLE derived.generations (
  id                    integer GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  state                 derived.generation_state NOT NULL,
  interpreter_versions  jsonb NOT NULL DEFAULT '{}',  -- version de chaque interprète au démarrage
  created_at            timestamptz NOT NULL DEFAULT now(),
  activated_at          timestamptz
);
CREATE UNIQUE INDEX generations_one_active ON derived.generations ((true)) WHERE state = 'active';
CREATE UNIQUE INDEX generations_one_building ON derived.generations ((true)) WHERE state = 'building';

-- Position de chaque interprète : dernier numéro du journal traité.
CREATE TABLE derived.cursors (
  generation  integer NOT NULL REFERENCES derived.generations ON DELETE CASCADE,
  kind        text NOT NULL REFERENCES core.kinds,
  last_seq    bigint NOT NULL DEFAULT 0,
  PRIMARY KEY (generation, kind)
);

-- Seuls les échecs sont stockés : le curseur dit déjà ce qui a été traité.
CREATE TABLE derived.interpretation_errors (
  generation           integer NOT NULL REFERENCES derived.generations ON DELETE CASCADE,
  seq                  bigint NOT NULL REFERENCES core.records,
  interpreter_version  integer NOT NULL,
  error                text NOT NULL,
  failed_at            timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (generation, seq)
);

-- Événements normalisés. Clé déterministe : deux générations se comparent par jointure.
CREATE TABLE derived.events (
  generation           integer NOT NULL REFERENCES derived.generations ON DELETE CASCADE,
  seq                  bigint NOT NULL REFERENCES core.records,
  ordinal              smallint NOT NULL,             -- un enregistrement donne 0, 1 ou n événements
  metric               text NOT NULL REFERENCES core.metrics,
  source_id            smallint NOT NULL REFERENCES core.sources,
  occurred_at          timestamptz NOT NULL,
  utc_offset_minutes   smallint,
  value                numeric,
  details              jsonb NOT NULL DEFAULT '{}',
  interpreter_version  integer NOT NULL,
  PRIMARY KEY (generation, seq, ordinal)
);
CREATE INDEX ON derived.events (generation, metric, occurred_at);
CREATE INDEX ON derived.events (generation, occurred_at);

-- Les éléments d'un événement (artiste, album, titre, contact…), un par niveau.
-- Générique : sert à Occurrences, Découverte, Dernière fois, Classement et Rechercher.
CREATE TABLE derived.event_items (
  generation   integer NOT NULL,
  seq          bigint NOT NULL,
  ordinal      smallint NOT NULL,
  level        text NOT NULL,
  item_key     text NOT NULL,   -- clé normalisée
  display      text NOT NULL,   -- tel qu'affiché par la source
  metric       text NOT NULL,   -- recopiés de l'événement pour l'indexation
  occurred_at  timestamptz NOT NULL,
  PRIMARY KEY (generation, seq, ordinal, level),
  FOREIGN KEY (generation, seq, ordinal) REFERENCES derived.events ON DELETE CASCADE
);
CREATE INDEX ON derived.event_items (generation, metric, level, item_key, occurred_at);
CREATE INDEX ON derived.event_items USING gin (item_key gin_trgm_ops);

-- Annulations : la cible est désignée par (source, clé), car elle peut arriver après l'annulation.
CREATE TABLE derived.cancellations (
  generation        integer NOT NULL REFERENCES derived.generations ON DELETE CASCADE,
  cancel_seq        bigint NOT NULL REFERENCES core.records,
  target_source_id  smallint NOT NULL,
  target_key        text NOT NULL,
  PRIMARY KEY (generation, cancel_seq)
);
CREATE INDEX ON derived.cancellations (generation, target_source_id, target_key);

-- Ce que lisent les vues : la génération active, sans les enregistrements annulés
-- ni les sources qui ne font pas foi sur la période.
CREATE VIEW derived.live_events AS
SELECT e.*
FROM derived.events e
JOIN derived.generations g ON g.id = e.generation AND g.state = 'active'
JOIN core.records r ON r.seq = e.seq
WHERE NOT EXISTS (
        SELECT 1 FROM derived.cancellations c
        WHERE c.generation = e.generation
          AND c.target_source_id = r.source_id
          AND c.target_key = r.key)
  AND NOT EXISTS (
        SELECT 1 FROM core.source_precedence p
        WHERE p.metric = e.metric
          AND p.source_id <> e.source_id
          AND p.valid_during @> e.occurred_at);

CREATE VIEW derived.live_event_items AS
SELECT i.*
FROM derived.event_items i
JOIN derived.live_events e USING (generation, seq, ordinal);

-- ─── Droits ──────────────────────────────────────────────────────────────────

GRANT USAGE ON SCHEMA core, derived TO tracker_app, tracker_admin;

-- tracker_app : ajoute au journal, ne le modifie jamais.
GRANT SELECT ON core.settings, core.kinds, core.sources, core.source_kinds, core.records,
  core.metrics, core.source_precedence, core.people, core.pseudonyms, core.account, core.sessions
  TO tracker_app;
GRANT INSERT ON core.records TO tracker_app;
GRANT UPDATE (last_seen_at, client_version, rejected_count, last_rejected_at) ON core.sources TO tracker_app;
GRANT INSERT, UPDATE, DELETE ON core.people, core.pseudonyms, core.sessions TO tracker_app;
GRANT UPDATE (username, password_hash, updated_at) ON core.account TO tracker_app;

-- tracker_admin : registres, purge (rédaction seule), reconstruction.
GRANT SELECT ON ALL TABLES IN SCHEMA core TO tracker_admin;
GRANT UPDATE (payload, purge_id) ON core.records TO tracker_admin;
GRANT INSERT ON core.purges TO tracker_admin;
GRANT INSERT, UPDATE ON core.kinds, core.metrics, core.sources, core.source_kinds, core.settings,
  core.account TO tracker_admin;
GRANT INSERT, UPDATE, DELETE ON core.source_precedence TO tracker_admin;
GRANT DELETE ON core.sessions TO tracker_admin;

-- derived : entièrement recalculable, lu et écrit par les deux.
GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA derived TO tracker_app, tracker_admin;
ALTER DEFAULT PRIVILEGES IN SCHEMA derived
  GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO tracker_app, tracker_admin;
