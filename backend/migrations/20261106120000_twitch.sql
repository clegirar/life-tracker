-- Twitch, par une extension Firefox (voir le journal des décisions de l'architecture, 6 nov. 2026) :
-- un type de données et sa métrique. La source `twitch` et son token sont créés par l'outil
-- d'administration : aucun secret dans le dépôt.

INSERT INTO core.kinds (kind, current_schema_version, description) VALUES
  ('twitch.watch', 1, 'Un segment de lecture Twitch mesuré par l''extension Firefox : chaîne (login et nom), contenu (live, vod, clip), durée en ms (1 h au plus), catégorie (liste blanche ; ni URL, ni titre, ni chat)');

-- La première métrique de forme `duration` : chaque événement est l'intervalle [début, début + durée
-- jouée] (`details.played_ms`), et le temps d'un jour est l'union de ces intervalles coupée au minuit
-- de Paris (deux streams en même temps comptent une fois). Calculé à la lecture (`read.rs`), aucune
-- vue matérialisée. Pas de seuil de fraîcheur : une semaine sans Twitch est normale, le silence de
-- l'extension se voit par celui de sa source.
INSERT INTO core.metrics
  (metric, label, shape, unit, domain, daily_aggregation, item_levels, discovery,
   applies_to, duration_from, freshness_threshold, description, display_order, shared_details)
VALUES
  ('stream.watch', 'Twitch', 'duration', 'min', 'vidéo', 'sum', '{channel}', true,
   NULL, NULL, NULL,
   'Le temps passé à regarder Twitch (live, rediffusion, clip), par chaîne ; un temps mesuré en lecture, deux streams en même temps comptent une fois dans le total',
   80, '{channel_name,content,category,played_ms}');
