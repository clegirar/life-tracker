-- La fréquence cardiaque de la nuit (voir le journal des décisions de l'architecture, 4 nov. 2026).
-- `garmin.sleep` garde son format 1 : `heart_rate` y est un champ facultatif, absent des nuits d'avant.
-- Les lectures génériques, donc le MCP, reçoivent la plus basse et la moyenne, jamais la série.
-- Une migration nouvelle : celles de Garmin sont déjà déployées.

UPDATE core.kinds
  SET description = 'Le résumé d''une nuit Garmin (durée, phases, début et fin, fréquence cardiaque), réduit à une liste blanche ; une révision annule la précédente'
  WHERE kind = 'garmin.sleep';

UPDATE core.metrics
  SET shared_details = shared_details || '{lowest_hr,avg_hr}',
      description = 'Le sommeil de la nuit, en minutes, rangé au jour du réveil ; détails : phases, début et fin, fréquence cardiaque la plus basse et moyenne'
  WHERE metric = 'sleep';
