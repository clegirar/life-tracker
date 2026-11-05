-- La recherche d'un élément par son nom affiché, et plus seulement par sa clé (voir le journal des
-- décisions de l'architecture, 5 nov. 2026). Les clés de YouTube sont des identifiants stables
-- (chaîne, vidéo) : « veritasium » ne s'y trouve pas, son nom affiché, si. Générique : les noms de la
-- musique ressemblent déjà à leurs clés. Seul le schéma de `derived` change : rien à migrer.

CREATE INDEX event_items_display_trgm ON derived.event_items USING gin (lower(display) gin_trgm_ops);
