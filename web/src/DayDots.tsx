// Les points d'une journée, façon Calendrier d'Apple, quand plusieurs métriques sont choisies : un point
// par métrique active ce jour-là (valeur > 0), dans sa couleur et l'ordre du registre. Ils disent la
// présence, pas l'intensité. La case du mois et le jour d'un mini-mois partagent ce rendu ; seules la
// taille et la limite changent : trois dans l'année, huit puis « +N » dans le mois.

import type { ReactNode } from 'react'
import type { MonthMetric } from './api'
import { metricTone, visibleDots } from './metrics'

const LIMITS = { year: 3, month: 8 } as const

export function DayDots({ active, size }: { active: string[]; size: 'year' | 'month' }) {
  const { dots, more } = visibleDots(active, LIMITS[size])
  return (
    <span className={`day-dots day-dots-${size}`} aria-hidden="true">
      {dots.map((metric) => (
        <span key={metric} className={`day-dot ${metricTone(metric)}`} data-metric={metric} />
      ))}
      {size === 'month' && more > 0 && <span className="day-dots-more">+{more}</span>}
    </span>
  )
}

interface LegendProps {
  metrics: MonthMetric[]
  /** `values` : les chiffres d'une seule métrique (le mois) ; `dots` : les points de plusieurs. */
  mode: 'values' | 'dots'
  /** Ce que la page ajoute : les marques du mois. */
  children?: ReactNode
}

const NOTES = {
  values: 'Sans chiffre : pas de données. 0 : rien ce jour-là.',
  dots: 'Un point par activité du jour. Numéro grisé : pas de données.',
}

/** La légende des chiffres et des points : la couleur de chaque métrique, puis ce que dit l'absence. */
export function MetricsLegend({ metrics, mode, children }: LegendProps) {
  return (
    <section className="legend" aria-label="Légende">
      <ul className="legend-list">
        {metrics.map((metric) => (
          <li key={metric.metric}>
            <span className={`${mode === 'dots' ? 'day-dot legend-day-dot' : 'legend-swatch'} ${metricTone(metric.metric)}`} aria-hidden="true" />
            {metric.label}
          </li>
        ))}
        {children}
      </ul>
      <p className="legend-note">{NOTES[mode]}</p>
    </section>
  )
}
