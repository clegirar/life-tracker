-- Steam, par l'API Web de Steam (voir le journal des décisions de l'architecture, 6 nov. 2026) : deux
-- types de données, deux métriques, la référence du total d'heures et la déclaration « déjà connu ». La source `steam` et son token
-- sont créés par l'outil d'administration : aucun secret dans le dépôt.

INSERT INTO core.kinds (kind, current_schema_version, description) VALUES
  ('steam.playtime',    1, 'Un relevé du compteur d''un jeu Steam (minutes depuis toujours), avec le relevé précédent du même jeu, que le connecteur garde : l''interprète en tire le temps joué entre les deux (liste blanche : appid, nom, compteurs, dernière partie)'),
  ('steam.achievement', 1, 'Un succès Steam débloqué, à sa date de déblocage : appid, nom du jeu, identifiant et nom du succès (sans description)');

-- Steam ne donne que des compteurs cumulés : le temps d'une partie est l'écart entre deux relevés,
-- placé juste avant la fin de la partie et découpé en tranches d'une heure au plus. Il prend la forme
-- `duration` de Twitch (union des intervalles, coupée au minuit de Paris). La clé d'un jeu est son
-- appid, qui ne change pas quand le jeu est renommé ; il s'affiche sous son nom. Un jeu joué pour la
-- première fois est une découverte ; un jeu déjà joué avant le suivi (premier relevé à plus de zéro)
-- est déclaré « déjà connu » par l'interprète (`game.known`), qui l'exclut des découvertes comme
-- `music.known` pour la musique. Pas de seuil de fraîcheur : une semaine sans jouer est normale, le
-- silence du collecteur se voit par celui de sa source. Les succès se lisent dans la carte Steam et
-- par les lectures génériques, sans valeur du jour (`none`) : ni case du calendrier ni filtre.
INSERT INTO core.metrics
  (metric, label, shape, unit, domain, daily_aggregation, item_levels, discovery,
   applies_to, duration_from, freshness_threshold, description, display_order, shared_details)
VALUES
  ('game.play', 'Jeux', 'duration', 'min', 'jeu vidéo', 'sum', '{game}', true,
   NULL, NULL, NULL,
   'Le temps joué sur Steam, par jeu ; déduit de l''écart entre deux relevés du compteur, placé à la fin de la partie, donc à quelques minutes près',
   90, '{played_ms}'),
  ('game.total_playtime', 'Total d''heures d''un jeu', 'reference', 'min', 'jeu vidéo', 'none', '{game}', false,
   'game.play', NULL, NULL,
   'Valeur : le total joué sur un jeu depuis toujours, en minutes, tel que Steam l''affiche ; le dernier relevé fait foi',
   NULL, '{}'),
  ('game.known', 'Déjà joué', 'declaration', NULL, 'jeu vidéo', 'none', '{game}', false,
   'game.play', NULL, NULL,
   'Exclut un jeu des découvertes : il avait déjà du temps de jeu au premier relevé, donc avant le suivi',
   NULL, '{}'),
  ('game.achievement', 'Succès', 'rich_event', 'succès', 'jeu vidéo', 'none', '{game,achievement}', false,
   NULL, NULL, NULL,
   'Un succès Steam débloqué, à sa vraie date, par jeu et par succès (clé `<appid>:<identifiant>`) ; sans valeur du jour, hors du calendrier',
   95, '{}');
