// Les filtres du calendrier : une pastille par métrique du registre, communes au mois et à l'année.
// Un nouveau tracker y apparaît de lui-même ; la page du jour pourra les reprendre.

import type { MonthMetric } from './api'
import { metricTone } from './metrics'

interface Props {
  metrics: MonthMetric[]
  selected: string[]
  onToggle: (metric: string) => void
}

export function MetricFilters({ metrics, selected, onToggle }: Props) {
  if (metrics.length === 0) return null
  return (
    <div className="filters" role="group" aria-label="Métriques affichées">
      {metrics.map((metric) => {
        const on = selected.includes(metric.metric)
        return (
          <button
            key={metric.metric}
            type="button"
            className={`chip ${metricTone(metric.metric)}`}
            aria-pressed={on}
            onClick={() => onToggle(metric.metric)}
          >
            <span className="chip-dot" aria-hidden="true" />
            {metric.label}
          </button>
        )
      })}
    </div>
  )
}

const LEVELS = [1, 2, 3, 4] as const

/** Une rampe : du moins au plus, dans la couleur de la métrique. */
function Ramp({ metric }: { metric: string }) {
  return (
    <span className={`ramp ${metricTone(metric)}`} aria-hidden="true">
      <span className="dot legend-dot" data-heat="0" />
      {LEVELS.map((level) => (
        <span key={level} className="dot legend-dot" data-heat={level} />
      ))}
    </span>
  )
}

/** La légende du mode intensité de l'année : une seule métrique, une rampe dans sa couleur. */
export function HeatLegend({ metric }: { metric: string }) {
  return (
    <section className="legend" aria-label="Légende">
      <ul className="legend-list">
        <li>
          <span className="dot legend-dot" data-heat="none" aria-hidden="true" />
          Pas de données
        </li>
        <li className={metricTone(metric)}>
          Rien
          <Ramp metric={metric} />
          Plus
        </li>
      </ul>
      <p className="legend-note">Vide : pas de données. Pâle : rien ce jour-là.</p>
    </section>
  )
}
