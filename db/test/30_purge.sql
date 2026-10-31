-- Purge exceptionnelle : rédaction en place, registre, dérivés supprimés, clé conservée.
BEGIN;

SELECT t.make_source('purge_src', '{coffee.cup}');
SELECT t.rec('purge_src', 'coffee.cup', 'a', '2026-10-17T07:00:00Z') AS seq_a \gset
SELECT t.rec('purge_src', 'coffee.cup', 'b', '2026-10-17T08:00:00Z') AS seq_b \gset
SELECT t.rec('purge_src', 'coffee.cup', 'c', '2026-10-17T09:00:00Z') AS seq_c \gset

INSERT INTO derived.generations (state) VALUES ('active') RETURNING id AS gen \gset
SELECT t.event(:gen, :seq_a, 'coffee'), t.event(:gen, :seq_b, 'coffee'), t.event(:gen, :seq_c, 'coffee');

SELECT md5(string_agg(r::text, ',' ORDER BY seq)) AS others_before
FROM core.records r WHERE seq <> :seq_b \gset

-- La purge, telle que l'exécutera l'outil d'administration (une seule transaction).
SELECT t.value_as('tracker_admin', format(
  $q$INSERT INTO core.purges (selector, reason, record_count) VALUES (%L, 'test', 1) RETURNING id$q$,
  jsonb_build_object('seq', :seq_b))) AS purge \gset
SELECT t.check(t.exec_as('tracker_admin',
  format('UPDATE core.records SET payload = NULL, purge_id = %s WHERE seq = %s', :purge, :seq_b)) = 1,
  'rédaction de l''enregistrement visé');
SELECT t.check(t.exec_as('tracker_admin', format('DELETE FROM derived.events WHERE seq = %s', :seq_b)) = 1,
  'suppression de ses événements dérivés');

SELECT t.check(md5(string_agg(r::text, ',' ORDER BY seq)) = :'others_before', 'les autres enregistrements sont intacts')
FROM core.records r WHERE seq <> :seq_b;
SELECT t.check(payload IS NULL AND purge_id = :purge, 'charge utile vidée, purge référencée')
FROM core.records WHERE seq = :seq_b;
SELECT t.check(key = 'b' AND occurred_at = '2026-10-17T08:00:00Z', 'enveloppe conservée')
FROM core.records WHERE seq = :seq_b;
SELECT t.check(count(*) = 1, 'entrée au registre des purges') FROM core.purges WHERE id = :purge;
SELECT t.check(count(*) = 2, 'les vues ne comptent plus que deux cafés')
FROM derived.live_events WHERE source_id = t.source_id('purge_src');
SELECT t.check(t.app_insert('purge_src', 'coffee.cup', 'b') = 0, 'la clé purgée reste un doublon : la donnée ne peut pas revenir');

ROLLBACK;
