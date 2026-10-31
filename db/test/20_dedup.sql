-- Dédoublonnage par (source, clé) et contrôles de l'enveloppe, sous le rôle tracker_app.
BEGIN;

SELECT t.make_source('dedup_a', '{coffee.cup}');
SELECT t.make_source('dedup_b', '{coffee.cup}');

SELECT t.check(t.app_insert('dedup_a', 'coffee.cup', 'click-1') = 1, 'premier envoi : accepté');
SELECT t.check(t.app_insert('dedup_a', 'coffee.cup', 'click-1') = 0, 'second envoi de la même clé : doublon, rien écrit');
SELECT t.check(t.app_insert('dedup_b', 'coffee.cup', 'click-1') = 1, 'même clé envoyée par une autre source : acceptée');

SELECT t.check(t.exec_as('tracker_app', $q$
  INSERT INTO core.records (source_id, kind, schema_version, key, occurred_at, payload)
  SELECT id, 'coffee.cup', 1, k, now(), '{}'
  FROM core.sources, unnest(ARRAY['batch-1', 'batch-2', 'batch-1', 'click-1']) AS k
  WHERE name = 'dedup_a'
  ON CONFLICT (source_id, key) DO NOTHING$q$) = 2,
  'lot renvoyé avec doublons internes et déjà reçus : seules les deux nouvelles clés entrent');

SELECT t.expect_error('tracker_app', t.insert_sql('dedup_a', 'lastfm.scrobble', 'm-1'),
                      '23503', 'type non déclaré pour la source');
SELECT t.expect_error('tracker_app', t.insert_sql('dedup_a', 'coffee.cup', 'null-payload', now(), NULL),
                      '23514', 'charge utile absente sans purge');
SELECT t.expect_error('tracker_app', t.insert_sql('dedup_a', 'coffee.cup', ''),
                      '23514', 'clé vide');

ROLLBACK;
