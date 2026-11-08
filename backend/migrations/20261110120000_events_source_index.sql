-- La couverture de chaque source, pour le calendrier et la santé des sources (voir le journal des
-- décisions de l'architecture, 8 nov. 2026) : la première et la dernière donnée vivante d'une métrique,
-- source par source, se lisent au bord de cet index, au lieu de parcourir tout l'historique à chaque
-- lecture. Seul le schéma de `derived` change : rien à migrer, rien à reconstruire.

CREATE INDEX events_generation_metric_source_occurred_at_idx
  ON derived.events (generation, metric, source_id, occurred_at);
