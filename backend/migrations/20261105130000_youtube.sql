-- YouTube, par Google Data Portability (voir le journal des décisions de l'architecture, 5 nov. 2026) :
-- deux types de données, une métrique et sa référence de durée. La source `youtube` et son token sont
-- créés par l'outil d'administration : aucun secret dans le dépôt.

INSERT INTO core.kinds (kind, current_schema_version, description) VALUES
  ('youtube.watch',      1, 'Une vidéo YouTube vue : id de la vidéo, titre, id et nom de la chaîne (liste blanche ; ni publicité, ni recherche, ni post, ni abonnement)'),
  ('youtube.video_info', 1, 'La fiche d''une vidéo tirée de l''API YouTube Data v3, dont sa durée (enrichissement)');

-- Les éléments ont pour clé les identifiants stables de YouTube (chaîne, vidéo) et pour nom ce que la
-- source affiche. La durée est celle de la vidéo, pas le temps réellement regardé : Google ne le donne pas.
INSERT INTO core.metrics
  (metric, label, shape, unit, domain, daily_aggregation, item_levels, discovery,
   applies_to, duration_from, freshness_threshold, description, display_order, shared_details)
VALUES
  ('video.watch', 'Vidéos YouTube', 'rich_event', 'vidéo', 'vidéo', 'count', '{channel,video}', true,
   NULL, 'video.duration', interval '3 days',
   'Une vidéo YouTube vue, par chaîne et par vidéo ; minutes estimées par la durée de la vidéo, pas le temps regardé',
   70, '{title,channel_name}'),
  ('video.duration', 'Durée d''une vidéo', 'reference', 's', 'vidéo', 'none', '{video}', false,
   'video.watch', NULL, NULL, 'Valeur : durée de la vidéo en secondes', NULL, '{}');
