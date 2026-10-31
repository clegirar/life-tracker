-- Registres du POC : réglages, types de données, métriques, source web app.
-- Les sources à token (connecteur Last.fm, raccourci iOS, import Apple) sont créées
-- par l'outil d'administration : aucun secret dans le dépôt.

INSERT INTO core.settings DEFAULT VALUES;

INSERT INTO core.kinds (kind, current_schema_version, description) VALUES
  ('coffee.cup',             1, 'Un café pris ; l''heure est celle de l''enveloppe, la charge utile est vide'),
  ('music.mark_known',       1, 'Déclaration « déjà connu » d''un artiste, d''un album ou d''un titre'),
  ('record.cancel',          1, 'Annulation d''un enregistrement, désigné par sa source et sa clé'),
  ('lastfm.scrobble',        1, 'Un scrobble tel que renvoyé par l''API Last.fm'),
  ('lastfm.track_info',      1, 'La fiche d''un titre renvoyée par Last.fm, dont sa durée (enrichissement)'),
  ('apple.play_history_row', 1, 'Une ligne du CSV d''historique d''écoute de l''export Apple');

-- applies_to et duration_from sont vérifiés en fin de transaction : l'ordre des lignes est libre.
INSERT INTO core.metrics
  (metric, label, shape, unit, domain, daily_aggregation, item_levels, discovery,
   applies_to, duration_from, freshness_threshold, description)
VALUES
  ('coffee', 'Cafés', 'counter', 'tasse', 'café', 'count', '{}', false,
   NULL, NULL, NULL, 'Un événement par café, daté à l''heure où il a été pris'),
  ('music.listen', 'Écoutes', 'rich_event', 'écoute', 'musique', 'count', '{artist,album,track}', true,
   NULL, 'music.track_duration', interval '3 days', 'Une écoute d''un titre ; minutes estimées par la durée du titre'),
  ('music.known', 'Déjà connu', 'declaration', NULL, 'musique', 'none', '{artist,album,track}', false,
   'music.listen', NULL, NULL, 'Exclut un élément des découvertes, à son seul niveau'),
  ('music.track_duration', 'Durée d''un titre', 'reference', 's', 'musique', 'none', '{track}', false,
   'music.listen', NULL, NULL, 'Valeur : durée du titre en secondes');

INSERT INTO core.sources (name, auth) VALUES ('webapp', 'session');

INSERT INTO core.source_kinds (source_id, kind)
SELECT s.id, k.kind
FROM core.sources s
CROSS JOIN (VALUES ('coffee.cup'), ('music.mark_known'), ('record.cancel')) AS k(kind)
WHERE s.name = 'webapp';
