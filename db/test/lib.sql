-- Outils de test, installés dans un schéma « t » de la base jetable (jamais dans une migration).
-- Appelés en superutilisateur ; exec_as et expect_error changent de rôle le temps d'une requête.
SET client_min_messages = warning;
DROP SCHEMA IF EXISTS t CASCADE;
CREATE SCHEMA t;

CREATE FUNCTION t.check(ok boolean, what text) RETURNS void LANGUAGE plpgsql AS $$
BEGIN
  IF ok IS NOT TRUE THEN
    RAISE EXCEPTION 'ÉCHEC : %', what;
  END IF;
  RAISE NOTICE 'ok  %', what;
END
$$;

-- Exécute q sous le rôle donné ; renvoie le nombre de lignes touchées.
CREATE FUNCTION t.exec_as(role text, q text) RETURNS bigint LANGUAGE plpgsql AS $$
DECLARE
  n bigint;
BEGIN
  EXECUTE format('SET LOCAL ROLE %I', role);
  EXECUTE q;
  GET DIAGNOSTICS n = ROW_COUNT;
  RESET ROLE;
  RETURN n;
END
$$;

-- Exécute q sous le rôle donné ; renvoie la première colonne de la première ligne.
CREATE FUNCTION t.value_as(role text, q text) RETURNS text LANGUAGE plpgsql AS $$
DECLARE
  v text;
BEGIN
  EXECUTE format('SET LOCAL ROLE %I', role);
  EXECUTE q INTO v;
  RESET ROLE;
  RETURN v;
END
$$;

-- Vérifie que q échoue sous le rôle donné, avec le code d'erreur attendu.
-- Le bloc d'exception annule tout ce que q a fait, changement de rôle compris.
CREATE FUNCTION t.expect_error(role text, q text, code text, what text) RETURNS void LANGUAGE plpgsql AS $$
BEGIN
  BEGIN
    EXECUTE format('SET LOCAL ROLE %I', role);
    EXECUTE q;
    RAISE EXCEPTION USING ERRCODE = 'TT000';
  EXCEPTION WHEN OTHERS THEN
    IF SQLSTATE = 'TT000' THEN
      RAISE EXCEPTION 'ÉCHEC : % : aurait dû échouer', what;
    END IF;
    IF SQLSTATE <> code THEN
      RAISE EXCEPTION 'ÉCHEC : % : attendu %, obtenu % (%)', what, code, SQLSTATE, SQLERRM;
    END IF;
  END;
  RAISE NOTICE 'ok  % (refusé, %)', what, code;
END
$$;

-- Une source à token autorisée pour les types donnés.
CREATE FUNCTION t.make_source(p_name text, p_kinds text[]) RETURNS smallint LANGUAGE sql AS $$
  WITH s AS (
    INSERT INTO core.sources (name, auth, token_hash)
    VALUES (p_name, 'token', sha256(convert_to(p_name, 'UTF8')))
    RETURNING id
  ), k AS (
    INSERT INTO core.source_kinds (source_id, kind) SELECT s.id, unnest(p_kinds) FROM s
  )
  SELECT id FROM s;
$$;

-- L'insertion telle que la fera l'ingestion : un doublon ne renvoie aucune ligne.
CREATE FUNCTION t.insert_sql(p_source text, p_kind text, p_key text,
                             p_at timestamptz DEFAULT now(), p_payload jsonb DEFAULT '{}') RETURNS text
LANGUAGE sql AS $$
  SELECT format(
    'INSERT INTO core.records (source_id, kind, schema_version, key, occurred_at, payload)
     SELECT id, %L, 1, %L, %L, %L FROM core.sources WHERE name = %L
     ON CONFLICT (source_id, key) DO NOTHING',
    p_kind, p_key, p_at, p_payload, p_source);
$$;

CREATE FUNCTION t.app_insert(p_source text, p_kind text, p_key text,
                             p_at timestamptz DEFAULT now(), p_payload jsonb DEFAULT '{}') RETURNS bigint
LANGUAGE sql AS $$
  SELECT t.exec_as('tracker_app', t.insert_sql(p_source, p_kind, p_key, p_at, p_payload));
$$;

-- Préparation des données, en superutilisateur.
CREATE FUNCTION t.rec(p_source text, p_kind text, p_key text, p_at timestamptz,
                      p_payload jsonb DEFAULT '{}') RETURNS bigint LANGUAGE sql AS $$
  INSERT INTO core.records (source_id, kind, schema_version, key, occurred_at, payload)
  SELECT id, p_kind, 1, p_key, p_at, p_payload FROM core.sources WHERE name = p_source
  RETURNING seq;
$$;

CREATE FUNCTION t.event(p_gen integer, p_seq bigint, p_metric text, p_details jsonb DEFAULT '{}')
RETURNS void LANGUAGE sql AS $$
  INSERT INTO derived.events
    (generation, seq, ordinal, metric, source_id, occurred_at, utc_offset_minutes, details, interpreter_version)
  SELECT p_gen, seq, 0, p_metric, source_id, occurred_at, utc_offset_minutes, p_details, 1
  FROM core.records WHERE seq = p_seq;
$$;

CREATE FUNCTION t.source_id(p_name text) RETURNS smallint LANGUAGE sql AS $$
  SELECT id FROM core.sources WHERE name = p_name;
$$;
