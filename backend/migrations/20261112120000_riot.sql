-- Riot (voir le chantier Riot et le journal des décisions de l'architecture, 12 nov. 2026) : League of
-- Legends et Teamfight Tactics par l'API officielle de Riot (source `riot`), VALORANT par l'API non
-- officielle HenrikDev (source `valorant`). Trois types de données, une métrique nouvelle. Les sources,
-- leurs tokens et les clés sont créés hors du dépôt : aucun secret ici.

INSERT INTO core.kinds (kind, current_schema_version, description) VALUES
  ('riot.lol_match', 1, 'Une partie de League of Legends, à son début : identifiant, début et fin, durée, file, mode, fin de partie, et ma ligne seulement (champion, victoire, poste, remake) ; jamais les autres joueurs'),
  ('riot.tft_match', 1, 'Une partie de Teamfight Tactics, à son début (`gameCreation`) : identifiant, début et fin, durée de la partie et mon temps de jeu (mon élimination), file, type, set, et ma ligne seulement (place, groupe de Double Up) ; jamais les autres joueurs'),
  ('valorant.match', 1, 'Une partie de VALORANT terminée, par HenrikDev, à son début : identifiant, début, durée, file, carte, et ma ligne seulement (agent, victoire de mon équipe, rounds gagnés et perdus) ; jamais les autres joueurs');

-- Le temps de jeu reprend `game.play` (forme `duration`) : chaque partie donne des tranches d'une heure
-- au plus depuis son début, sous l'élément `game` du jeu. Les clés des trois jeux portent le préfixe
-- `riot:` (`riot:lol`, `riot:tft`, `riot:valorant`), qu'aucun appid de Steam (un entier) ne peut
-- prendre. Ces jeux étaient joués bien avant le suivi : chaque partie les déclare aussi « déjà connus »
-- (`game.known`), ils ne sont jamais des découvertes. Sans ligne de `core.source_precedence` pour
-- `game.play`, Steam et Riot s'additionnent : ce sont des jeux différents, c'est voulu.
UPDATE core.metrics
SET description = 'Le temps joué, par jeu : sur Steam, déduit de l''écart entre deux relevés du compteur, placé à la fin de la partie, donc à quelques minutes près ; pour League of Legends, Teamfight Tactics (jusqu''à mon élimination) et VALORANT, la durée de chaque partie, depuis son début'
WHERE metric = 'game.play';

-- Une partie, sans valeur du jour (`none`, comme les succès Steam) : ni case du calendrier ni pastille
-- ni place dans `day_summary` ; la carte « Jeux » du jour les montre sous leur jeu, le MCP les lit par
-- `occurrences` et `top` (revu le 12 nov. 2026, à ma demande : un seul filtre « Jeux »). Éléments : le jeu, et le
-- personnage joué (champion de LoL sous `lol:<nom>`, agent de VALORANT sous `valorant:<nom>` ; aucun en
-- TFT). Détails : jeu, file (nom en clair, d'une table figée dans l'interprète), mode, résultat (`win`,
-- `loss`, `remake`, `draw`, ou rien), personnage, poste, carte, place en TFT, rounds, temps joué. Le
-- MCP n'en voit que le jeu, la file, le mode, le résultat, le personnage et la place.
INSERT INTO core.metrics
  (metric, label, shape, unit, domain, daily_aggregation, item_levels, discovery,
   applies_to, duration_from, freshness_threshold, description, display_order, shared_details)
VALUES
  ('game.match', 'Parties', 'rich_event', 'partie', 'jeu vidéo', 'none', '{game,character}', false,
   NULL, NULL, NULL,
   'Une partie de League of Legends, Teamfight Tactics ou VALORANT, à son début, par jeu et par personnage (champion ou agent) ; sans valeur du jour, hors du calendrier ; détails : file, mode, résultat (win, loss, remake, draw), place en TFT, carte, rounds, temps joué',
   92, '{game,queue,mode,result,character,placement}');
