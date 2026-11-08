-- GitHub, par l'API GraphQL de GitHub (voir le chantier GitHub, 7 nov. 2026) : cinq types de données et
-- six métriques du domaine « code ». La source `github` et son token sont créés par l'outil
-- d'administration, le token d'accès personnel vit sur le NAS : aucun secret dans le dépôt.

INSERT INTO core.kinds (kind, current_schema_version, description) VALUES
  ('github.commit',       1, 'Un commit de la branche par défaut d''un dépôt, dont je suis l''auteur, à sa date d''auteur : dépôt (id et nom), sha, dates d''auteur et de commit, première ligne du message, lignes ajoutées et supprimées, url (liste blanche ; jamais le corps du message)'),
  ('github.pull_request', 1, 'Une pull request que j''ai ouverte (à sa création) ou fusionnée (à sa fusion, avec ses lignes) : id, action, dépôt, numéro, titre, url, lignes à la fusion'),
  ('github.review',       1, 'Une review soumise sur la pull request d''un autre, à sa soumission : id, état, dépôt, numéro et titre de la pull request, url'),
  ('github.issue',        1, 'Une issue que j''ai ouverte, à sa création : id, dépôt, numéro, titre, url (jamais le corps)'),
  ('github.comment',      1, 'Un commentaire sur une issue, une pull request ou un commit, à sa création : id, dépôt, cible, numéro et titre de l''issue ou de la pull request, url (jamais le corps)');

-- L'élément `repo` est le dépôt, sous son id GraphQL, qui ne change pas quand le dépôt est renommé ou
-- transféré ; il s'affiche `propriétaire/nom`, le nom au moment du fait. Le premier commit sur un dépôt
-- est une découverte : l'historique étant collecté en entier, pas de déclaration « déjà connu ».
--
-- Une pull request donne deux faits, son ouverture et sa fusion, à deux dates. Une seule métrique
-- comptée (`count`) compterait chaque pull request fusionnée deux fois dans le calendrier, et un filtre
-- sur un détail n'existe pas dans les lectures génériques. La fusion a donc sa métrique,
-- `code.pull_request_merged`, sans valeur du jour (`none`) : le calendrier compte les ouvertures, la
-- carte du jour et les lectures génériques (MCP) lisent aussi les fusions, avec leurs lignes.
--
-- Reviews, issues et commentaires sont sans valeur du jour (`none`), comme les succès Steam : ni case
-- du calendrier ni filtre, lus dans la carte et par les lectures génériques. Les titres restent dans les
-- détails pour la carte, jamais dans `shared_details` : le MCP voit dépôts, comptes et lignes. Pas de
-- seuil de fraîcheur : une semaine sans coder est normale, le silence du collecteur se voit par celui de
-- sa source.
INSERT INTO core.metrics
  (metric, label, shape, unit, domain, daily_aggregation, item_levels, discovery,
   applies_to, duration_from, freshness_threshold, description, display_order, shared_details)
VALUES
  ('code.commit', 'Commits', 'rich_event', 'commit', 'code', 'count', '{repo}', true,
   NULL, NULL, NULL,
   'Un commit sur la branche par défaut d''un dépôt GitHub, à sa date d''auteur, par dépôt ; détails : sha, titre, lignes ajoutées et supprimées, url',
   110, '{additions,deletions}'),
  ('code.pull_request', 'Pull requests', 'rich_event', 'pull request', 'code', 'count', '{repo}', false,
   NULL, NULL, NULL,
   'Une pull request ouverte sur GitHub, à sa création, par dépôt ; détails : numéro, titre, url',
   120, '{}'),
  ('code.pull_request_merged', 'Pull requests fusionnées', 'rich_event', 'pull request', 'code', 'none', '{repo}', false,
   NULL, NULL, NULL,
   'Une de mes pull requests fusionnée, à sa fusion, par dépôt ; détails : numéro, titre, url, lignes ajoutées et supprimées ; sans valeur du jour, hors du calendrier',
   125, '{additions,deletions}'),
  ('code.review', 'Reviews', 'rich_event', 'review', 'code', 'none', '{repo}', false,
   NULL, NULL, NULL,
   'Une review soumise sur la pull request d''un autre, par dépôt ; détails : état (approved, changes_requested, commented, dismissed), numéro, titre, url ; sans valeur du jour, hors du calendrier',
   130, '{state}'),
  ('code.issue', 'Issues', 'rich_event', 'issue', 'code', 'none', '{repo}', false,
   NULL, NULL, NULL,
   'Une issue ouverte sur GitHub, à sa création, par dépôt ; détails : numéro, titre, url ; sans valeur du jour, hors du calendrier',
   140, '{}'),
  ('code.comment', 'Commentaires GitHub', 'rich_event', 'commentaire', 'code', 'none', '{repo}', false,
   NULL, NULL, NULL,
   'Un commentaire sur une issue, une pull request ou un commit, par dépôt ; détails : cible (issue, pull_request, commit), numéro et titre de la cible, url ; sans valeur du jour, hors du calendrier',
   150, '{target}');
