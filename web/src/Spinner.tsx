// L'indicateur d'une période en lecture, à côté de son titre : huit traits qui tournent par crans, à la
// manière d'iOS. Il n'apparaît qu'après 300 ms (CSS) : une lecture rapide ne clignote pas. La grille,
// elle, porte aria-busy ; l'indicateur est décoratif.

const TICKS = Array.from({ length: 8 }, (_, i) => i)

export function Spinner() {
  return (
    <svg className="spinner" viewBox="0 0 24 24" width="20" height="20" aria-hidden="true" data-testid="spinner">
      {TICKS.map((i) => (
        <rect
          key={i}
          x="11"
          y="2"
          width="2"
          height="6"
          rx="1"
          transform={`rotate(${i * 45} 12 12)`}
          opacity={1 - i * 0.1}
        />
      ))}
    </svg>
  )
}
