-- Garmin Connect, premier tracker après le POC (voir le journal des décisions de l'architecture,
-- 4 nov. 2026) : quatre types de données, quatre métriques et une référence (la trace d'une séance).
-- La source `garmin` et son token sont créés par l'outil d'administration : aucun secret dans le dépôt.

INSERT INTO core.kinds (kind, current_schema_version, description) VALUES
  ('garmin.daily_summary', 1, 'Le résumé d''une journée Garmin (pas, distance, calories), réduit à une liste blanche ; une révision annule la précédente'),
  ('garmin.sleep',         1, 'Le résumé d''une nuit Garmin (durée, phases, début et fin), réduit à une liste blanche ; une révision annule la précédente'),
  ('garmin.activity',      1, 'Le résumé d''une séance Garmin (type, durée, distance, allure, cardio, calories, début et fin, lieu), sans nom ni propriétaire ; une révision annule la précédente'),
  ('garmin.activity_track', 1, 'La trace simplifiée d''une séance, tirée de son fichier FIT : points (latitude, longitude, secondes depuis le début)');

-- Un total du jour est une mesure datée (règle 9) : la dernière version vivante fait foi (`last`).
-- Le sommeil est une mesure en minutes et non une durée : il a des versions, et une durée exigerait
-- une référence (`duration_from`) qui n'a pas de sens ici.
INSERT INTO core.metrics
  (metric, label, shape, unit, domain, daily_aggregation, item_levels, discovery,
   applies_to, duration_from, freshness_threshold, description)
VALUES
  ('steps', 'Pas', 'measure', 'pas', 'activité', 'last', '{}', false,
   NULL, NULL, interval '3 days', 'Le nombre de pas du jour selon Garmin ; détails : distance en mètres'),
  ('calories.burned', 'Calories brûlées', 'measure', 'kcal', 'activité', 'last', '{}', false,
   NULL, NULL, NULL, 'Les calories brûlées du jour selon Garmin ; détails : actives et au repos'),
  ('sleep', 'Sommeil', 'measure', 'min', 'activité', 'last', '{}', false,
   NULL, NULL, interval '3 days', 'Le sommeil de la nuit, en minutes, rangé au jour du réveil ; détails : phases, début et fin'),
  ('workout', 'Séances', 'rich_event', 'séance', 'activité', 'count', '{activity_type,place}', false,
   NULL, NULL, NULL, 'Une séance (course, marche, vélo…), par type et par lieu (la ville selon Garmin) ; valeur : distance en km'),
  -- Les coordonnées ne sortent que par les routes de session de la web app : jamais par le MCP.
  ('workout.track', 'Trace d''une séance', 'reference', NULL, 'activité', 'none', '{}', false,
   'workout', NULL, NULL, 'La trace simplifiée d''une séance ; détails : identifiant de la séance et points');
