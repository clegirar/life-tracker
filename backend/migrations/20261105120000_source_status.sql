-- La page d'état des trackers (voir le journal des décisions de l'architecture, 5 nov. 2026).
-- `status` : le dernier état rapporté par la source (`PUT /v1/sources/me/status`), un objet au
-- vocabulaire fermé validé par l'API ; jamais de texte libre. Hors du journal, comme le dernier contact :
-- seul le dernier compte, et le rapporter ne touche pas à `last_seen_at`.
-- `label` : le nom affiché (« Garmin Connect »), fixé par l'outil d'administration ; vide, le nom.

ALTER TABLE core.sources
  ADD COLUMN status jsonb CHECK (jsonb_typeof(status) = 'object'),
  ADD COLUMN status_reported_at timestamptz,
  ADD COLUMN label text CHECK (label = btrim(label) AND char_length(label) BETWEEN 1 AND 80);

GRANT UPDATE (status, status_reported_at) ON core.sources TO tracker_app;
