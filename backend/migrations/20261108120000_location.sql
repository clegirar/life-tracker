-- La position de l'iPhone, par l'app Overland (voir le journal des décisions de l'architecture,
-- 7 nov. 2026) : deux types de données et deux métriques. Overland envoie ses lots à une route
-- d'adaptation (`POST /v1/locations/overland`), qui les traduit en enregistrements de l'API
-- d'ingestion. La source `iphone` et son token sont créés par l'outil d'administration : aucun secret
-- dans le dépôt.

INSERT INTO core.kinds (kind, current_schema_version, description) VALUES
  ('overland.location', 1, 'Un point de position de l''iPhone envoyé par Overland : latitude, longitude, précision, altitude, vitesse, mouvement (liste blanche ; ni Wi-Fi, ni batterie, ni identifiant d''appareil)'),
  ('overland.visit',    1, 'Une visite détectée par iOS et envoyée par Overland, une fois son départ connu : latitude, longitude, arrivée, départ, précision');

-- `location.point` : un point brut, que seule la carte du jour lit, par une route de session. Une
-- mesure sans valeur du jour (`none`) ni ordre d'affichage : ni case du calendrier, ni filtre, et
-- aucun détail partagé, donc aucune coordonnée pour le MCP. Une référence aurait demandé une
-- métrique à éclairer (`applies_to`), et un point n'en éclaire aucune.
-- `place.visit` : un lieu où je suis resté, à son arrivée ; la valeur du jour est le nombre de lieux.
-- Ses coordonnées (`visit_lat`, `visit_lon`) passent par les zones de confidentialité pour le jeton
-- de service. Pas de nom de lieu : il demandera un géocodage inverse, reporté à un chantier suivant.
INSERT INTO core.metrics
  (metric, label, shape, unit, domain, daily_aggregation, item_levels, discovery,
   applies_to, duration_from, freshness_threshold, description, display_order, shared_details)
VALUES
  ('location.point', 'Position', 'measure', NULL, 'déplacements', 'none', '{}', false,
   NULL, NULL, NULL,
   'Un point de position de l''iPhone (Overland), lu par la seule carte du jour ; détails : coordonnées, précision, altitude, vitesse, mouvement',
   NULL, '{}'),
  ('place.visit', 'Déplacements', 'rich_event', 'lieu', 'déplacements', 'count', '{}', false,
   NULL, NULL, NULL,
   'Un lieu où je suis resté, détecté par iOS, à son arrivée ; détails : coordonnées, arrivée, départ, durée en secondes',
   100, '{visit_lat,visit_lon,arrival,departure,duration_s}');
