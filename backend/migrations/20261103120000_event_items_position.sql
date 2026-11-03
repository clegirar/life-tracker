-- Plusieurs éléments d'un même niveau par événement : une écoute crédite chacun de ses artistes
-- (voir le journal des décisions de l'architecture, 3 nov. 2026). `position` les range : 0 pour le
-- premier, l'artiste principal d'une écoute ; tout autre niveau n'a qu'un élément, en 0.
-- Seul le schéma de `derived` change : `admin rebuild` le remplit, ses données ne se migrent jamais.

ALTER TABLE derived.event_items
  ADD COLUMN position smallint NOT NULL DEFAULT 0 CHECK (position >= 0);
ALTER TABLE derived.event_items
  DROP CONSTRAINT event_items_pkey,
  ADD PRIMARY KEY (generation, seq, ordinal, level, position);

-- `i.*` est figé à la création de la vue : la recréer y ajoute la colonne, à la fin.
CREATE OR REPLACE VIEW derived.live_event_items AS
SELECT i.*
FROM derived.event_items i
JOIN derived.live_events e USING (generation, seq, ordinal);
