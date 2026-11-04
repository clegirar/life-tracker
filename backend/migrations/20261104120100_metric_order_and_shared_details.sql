-- Deux colonnes du registre des métriques (voir le journal des décisions de l'architecture, 4 nov. 2026).
-- Une migration à part plutôt que dans celle de Garmin : `core.metrics` vient du schéma initial, déjà
-- déployé, et ces colonnes valent pour toutes les métriques.
--
-- `display_order` : l'ordre d'affichage des métriques (filtres et légendes du calendrier, inventaire
-- du MCP). Vide : après les autres, par nom.
-- `shared_details` : les champs des détails qu'une lecture générique peut rendre (occurrences, donc
-- le MCP). Vide : aucun, comme pour le café et la musique. Une paire `<x>_lat`/`<x>_lon` y est une
-- coordonnée : le jeton de service ne la reçoit que hors des zones de confidentialité.

ALTER TABLE core.metrics
  ADD COLUMN display_order smallint,
  ADD COLUMN shared_details text[] NOT NULL DEFAULT '{}';

UPDATE core.metrics SET display_order = 10 WHERE metric = 'coffee';
UPDATE core.metrics SET display_order = 20 WHERE metric = 'music.listen';
UPDATE core.metrics SET display_order = 30, shared_details = '{distance_m}' WHERE metric = 'steps';
UPDATE core.metrics SET display_order = 40,
  shared_details = '{activity_id,type,duration_s,moving_s,distance_m,avg_speed_mps,avg_hr,max_hr,calories,location_name,start_lat,start_lon,end_lat,end_lon}'
  WHERE metric = 'workout';
UPDATE core.metrics SET display_order = 50,
  shared_details = '{start,end,sleep_seconds,deep_seconds,light_seconds,rem_seconds,awake_seconds}'
  WHERE metric = 'sleep';
UPDATE core.metrics SET display_order = 60, shared_details = '{active,resting}' WHERE metric = 'calories.burned';
