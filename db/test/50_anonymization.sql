-- Supprimer une personne de l'annuaire anonymise son historique sans toucher au journal.
BEGIN;

-- Les messages sont hors POC : le test déclare son propre type et sa propre métrique.
INSERT INTO core.kinds (kind, current_schema_version, description)
VALUES ('test.message', 1, 'Message de test');
INSERT INTO core.metrics (metric, label, shape, unit, domain, daily_aggregation, item_levels, discovery, description)
VALUES ('test.message', 'Messages de test', 'rich_event', 'message', 'test', 'count', '{contact}', false, 'Test');

SELECT t.make_source('mac_test', '{test.message}');
SELECT t.exec_as('tracker_app', $q$
  WITH p AS (INSERT INTO core.people (display_name) VALUES ('Alice') RETURNING id)
  INSERT INTO core.pseudonyms (pseudonym, person_id)
  SELECT unnest(ARRAY['ps_phone', 'ps_mail']), id FROM p$q$);

SELECT t.rec('mac_test', 'test.message', 'guid-1', '2026-10-17T10:00:00Z',
             '{"direction": "sent", "service": "imessage", "contact": "ps_phone"}') AS m1 \gset
SELECT t.rec('mac_test', 'test.message', 'guid-2', '2026-10-17T11:00:00Z',
             '{"direction": "received", "service": "sms", "contact": "ps_mail"}') AS m2 \gset

INSERT INTO derived.generations (state) VALUES ('active') RETURNING id AS gen \gset
SELECT t.event(:gen, :m1, 'test.message', '{"direction": "sent", "service": "imessage"}'),
       t.event(:gen, :m2, 'test.message', '{"direction": "received", "service": "sms"}');
INSERT INTO derived.event_items (generation, seq, ordinal, level, item_key, display, metric, occurred_at)
SELECT generation, seq, ordinal, 'contact', r.payload->>'contact', r.payload->>'contact', metric, e.occurred_at
FROM derived.events e JOIN core.records r USING (seq)
WHERE e.generation = :gen AND seq IN (:m1, :m2);

SELECT t.check(count(*) = 2, 'avant : les deux messages sont rattachés à Alice')
FROM derived.live_event_items i
JOIN core.pseudonyms p ON p.pseudonym = i.item_key
JOIN core.people pe ON pe.id = p.person_id
WHERE pe.display_name = 'Alice';

SELECT md5(string_agg(r::text, ',' ORDER BY seq)) AS records_before FROM core.records r \gset
SELECT md5(string_agg(e::text, ',' ORDER BY seq, ordinal)) AS events_before FROM derived.events e \gset

SELECT t.check(t.exec_as('tracker_app', $q$DELETE FROM core.people WHERE display_name = 'Alice'$q$) = 1,
               'suppression de la personne dans l''annuaire');

SELECT t.check(count(*) = 0, 'ses pseudonymes sont supprimés avec elle')
FROM core.pseudonyms WHERE pseudonym IN ('ps_phone', 'ps_mail');
SELECT t.check(md5(string_agg(r::text, ',' ORDER BY seq)) = :'records_before', 'journal intact') FROM core.records r;
SELECT t.check(md5(string_agg(e::text, ',' ORDER BY seq, ordinal)) = :'events_before', 'événements intacts') FROM derived.events e;
SELECT t.check(count(*) = 2 AND count(p.pseudonym) = 0, 'les messages restent comptés, mais plus aucun nom n''y est rattaché')
FROM derived.live_event_items i
LEFT JOIN core.pseudonyms p ON p.pseudonym = i.item_key
WHERE i.seq IN (:m1, :m2);

ROLLBACK;
