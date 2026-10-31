-- Journées à l'heure de Paris calculées à la lecture, y compris au passage à l'heure d'hiver ;
-- heure locale d'un café.
BEGIN;

SELECT t.make_source('dst_src', '{coffee.cup}');

-- Un café pris à 8h15 heure de Paris, envoyé avec son décalage horaire.
SELECT t.value_as('tracker_app', $q$
  INSERT INTO core.records (source_id, kind, schema_version, key, occurred_at, utc_offset_minutes, payload)
  SELECT id, 'coffee.cup', 1, 'morning', '2026-10-17T08:15:00+02:00', 120, '{}'
  FROM core.sources WHERE name = 'dst_src'
  RETURNING seq$q$) AS morning \gset

-- d1 : 00:30 le 25, heure d'été ; d2 : 23:30 le 25, heure d'hiver ; d3 : 00:30 le 26.
SELECT t.rec('dst_src', 'coffee.cup', 'd1', '2026-10-24T22:30:00Z') AS d1 \gset
SELECT t.rec('dst_src', 'coffee.cup', 'd2', '2026-10-25T22:30:00Z') AS d2 \gset
SELECT t.rec('dst_src', 'coffee.cup', 'd3', '2026-10-25T23:30:00Z') AS d3 \gset

INSERT INTO derived.generations (state) VALUES ('active') RETURNING id AS gen \gset
SELECT t.event(:gen, :morning, 'coffee'), t.event(:gen, :d1, 'coffee'),
       t.event(:gen, :d2, 'coffee'), t.event(:gen, :d3, 'coffee');

SELECT t.check(
  extract(epoch FROM ('2026-10-26'::timestamp AT TIME ZONE timezone) - ('2026-10-25'::timestamp AT TIME ZONE timezone)) = 25 * 3600,
  'le 25 oct. 2026 dure 25 heures à Paris')
FROM core.settings;

-- La requête type d'une journée : le jour demandé devient un intervalle UTC.
SELECT t.check(array_agg(r.key ORDER BY e.occurred_at) = ARRAY['d1', 'd2'],
               'le 25 oct. compte le café de 00:30 (heure d''été) et celui de 23:30 (heure d''hiver)')
FROM derived.live_events e
JOIN core.records r ON r.seq = e.seq
CROSS JOIN core.settings s
WHERE e.metric = 'coffee' AND e.source_id = t.source_id('dst_src')
  AND e.occurred_at >= ('2026-10-25'::timestamp AT TIME ZONE s.timezone)
  AND e.occurred_at <  ('2026-10-26'::timestamp AT TIME ZONE s.timezone);

SELECT t.check(min((e.occurred_at AT TIME ZONE s.timezone)::time) = '00:30', 'heure locale du premier café du 25 : 00:30')
FROM derived.live_events e CROSS JOIN core.settings s
WHERE e.source_id = t.source_id('dst_src')
  AND e.occurred_at >= ('2026-10-25'::timestamp AT TIME ZONE s.timezone)
  AND e.occurred_at <  ('2026-10-26'::timestamp AT TIME ZONE s.timezone);

SELECT t.check((occurred_at AT TIME ZONE 'Europe/Paris')::time = '08:15' AND utc_offset_minutes = 120,
               'un café garde son heure locale (8h15) et son décalage d''origine')
FROM derived.live_events WHERE seq = :morning;

ROLLBACK;
