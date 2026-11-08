// La place d'une carte du jour qui se lit : des barres immobiles, à peu près à la hauteur habituelle de
// son contenu, pour que la maçonnerie ne saute pas à l'arrivée des données. Pas de scintillement : la
// lecture dure rarement plus d'une seconde, et une page calme se lit mieux. Le contenu, lui, se fond.

/** `rows` : des lignes de 32 px (une barre et l'écart qui la suit). */
export function CardSkeleton({ rows }: { rows: number }) {
  return (
    <div className="skeleton" role="status" data-testid="card-loading">
      <span className="visually-hidden">Chargement…</span>
      {Array.from({ length: rows }, (_, index) => (
        <span key={index} className="skeleton-bar" aria-hidden="true" />
      ))}
    </div>
  )
}
