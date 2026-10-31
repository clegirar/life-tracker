-- Générations dérivées (construction à côté, bascule, nettoyage), annulations et priorité entre sources.
BEGIN;

SELECT t.make_source('gen_src', '{coffee.cup,record.cancel}');
SELECT t.rec('gen_src', 'coffee.cup', 'c1', '2026-10-17T07:00:00Z') AS c1 \gset
SELECT t.rec('gen_src', 'coffee.cup', 'c2', '2026-10-17T08:00:00Z') AS c2 \gset
-- L'annulation arrive avant sa cible (raccourci iOS resté hors ligne, par exemple).
SELECT t.rec('gen_src', 'record.cancel', 'x1', '2026-10-17T09:05:00Z', '{"source": "gen_src", "key": "c3"}') AS x1 \gset
SELECT t.rec('gen_src', 'coffee.cup', 'c3', '2026-10-17T09:00:00Z') AS c3 \gset

SELECT t.value_as('tracker_app',
  $q$INSERT INTO derived.generations (state, activated_at) VALUES ('active', now()) RETURNING id$q$) AS g1 \gset
SELECT t.event(:g1, :c1, 'coffee'), t.event(:g1, :c2, 'coffee'), t.event(:g1, :c3, 'coffee');
INSERT INTO derived.cancellations VALUES (:g1, :x1, t.source_id('gen_src'), 'c3');
INSERT INTO derived.cursors (generation, kind, last_seq) VALUES (:g1, 'coffee.cup', :c3), (:g1, 'record.cancel', :x1);

SELECT t.check(count(*) = 2, 'annulation reçue avant sa cible : la cible est masquée')
FROM derived.live_events WHERE source_id = t.source_id('gen_src');
SELECT t.expect_error('tracker_app', $q$INSERT INTO derived.generations (state) VALUES ('active')$q$,
                      '23505', 'une seule génération active');

-- Génération suivante, construite à côté (ici, un interprète corrigé ne garde que c1).
SELECT t.value_as('tracker_app',
  $q$INSERT INTO derived.generations (state) VALUES ('building') RETURNING id$q$) AS g2 \gset
SELECT t.expect_error('tracker_app', $q$INSERT INTO derived.generations (state) VALUES ('building')$q$,
                      '23505', 'une seule génération en construction');
SELECT t.event(:g2, :c1, 'coffee');

SELECT t.check(count(*) = 2 AND min(generation) = :g1, 'pendant la construction, les vues lisent l''ancienne génération')
FROM derived.live_events WHERE source_id = t.source_id('gen_src');
SELECT t.check(count(*) = 2, 'comparaison des deux générations par jointure : deux écarts')
FROM (
  SELECT seq, ordinal, metric, occurred_at, value, details FROM derived.events WHERE generation = :g1
  EXCEPT
  SELECT seq, ordinal, metric, occurred_at, value, details FROM derived.events WHERE generation = :g2
) AS diff;

-- Bascule : deux mises à jour dans la même transaction.
SELECT t.exec_as('tracker_app', format('UPDATE derived.generations SET state = ''retired'' WHERE id = %s', :g1));
SELECT t.exec_as('tracker_app', format('UPDATE derived.generations SET state = ''active'', activated_at = now() WHERE id = %s', :g2));
SELECT t.check(count(*) = 1 AND min(generation) = :g2, 'après la bascule, les vues lisent la nouvelle génération')
FROM derived.live_events WHERE source_id = t.source_id('gen_src');

SELECT t.exec_as('tracker_app', format('DELETE FROM derived.generations WHERE id = %s', :g1));
SELECT t.check(
  (SELECT count(*) FROM derived.events WHERE generation = :g1)
  + (SELECT count(*) FROM derived.cancellations WHERE generation = :g1)
  + (SELECT count(*) FROM derived.cursors WHERE generation = :g1) = 0,
  'supprimer l''ancienne génération supprime toutes ses lignes');

-- Priorité entre sources : Last.fm fait foi à partir de son démarrage.
SELECT t.make_source('lastfm_test', '{lastfm.scrobble}');
SELECT t.make_source('apple_test', '{apple.play_history_row}');
SELECT t.rec('apple_test', 'apple.play_history_row', 'a-before', '2026-09-15T10:00:00Z') AS a_before \gset
SELECT t.rec('apple_test', 'apple.play_history_row', 'a-overlap', '2026-10-05T10:00:00Z') AS a_overlap \gset
SELECT t.rec('lastfm_test', 'lastfm.scrobble', 'l-overlap', '2026-10-05T10:00:30Z') AS l_overlap \gset
SELECT t.event(:g2, :a_before, 'music.listen'), t.event(:g2, :a_overlap, 'music.listen'), t.event(:g2, :l_overlap, 'music.listen');

SELECT t.exec_as('tracker_admin', $q$
  INSERT INTO core.source_precedence (metric, source_id, valid_during)
  SELECT 'music.listen', id, tstzrange('2026-10-01T00:00:00Z', NULL) FROM core.sources WHERE name = 'lastfm_test'$q$);
SELECT t.check(array_agg(r.key ORDER BY r.key) = ARRAY['a-before', 'l-overlap'],
               'Last.fm fait foi sur le chevauchement, l''export Apple reste avant')
FROM derived.live_events e JOIN core.records r ON r.seq = e.seq
WHERE e.metric = 'music.listen';
SELECT t.expect_error('tracker_admin', $q$
  INSERT INTO core.source_precedence (metric, source_id, valid_during)
  SELECT 'music.listen', id, tstzrange('2026-10-10T00:00:00Z', '2026-10-20T00:00:00Z')
  FROM core.sources WHERE name = 'apple_test'$q$,
  '23P01', 'deux sources prioritaires sur la même période');

ROLLBACK;
