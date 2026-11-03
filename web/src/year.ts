// La vue de l'année : douze mini-mois, une pastille par jour dont l'intensité suit la valeur.
// Une pastille vide n'a pas de données ; une pastille pâle est un vrai zéro. Jamais l'un pour l'autre.

import type { MonthDay, MonthMetric } from './api'
import { monthName, valueText } from './calendar'

/** Les douze mois `AAAA-MM` d'une année. */
export function yearMonths(year: string): string[] {
  return Array.from({ length: 12 }, (_, i) => `${year}-${String(i + 1).padStart(2, '0')}`)
}

function quantile(sorted: number[], p: number): number {
  const position = p * (sorted.length - 1)
  const low = Math.floor(position)
  const high = Math.ceil(position)
  return sorted[low] + (sorted[high] - sorted[low]) * (position - low)
}

/** Les seuils des paliers : les quartiles des valeurs non nulles de l'année, puis son maximum. */
export function heatThresholds(values: (number | null)[]): number[] {
  const positive = values.filter((v): v is number => v !== null && v > 0).sort((a, b) => a - b)
  if (positive.length === 0) return []
  return [quantile(positive, 0.25), quantile(positive, 0.5), quantile(positive, 0.75), positive[positive.length - 1]]
}

/** `none` : pas de données ; 0 : rien ce jour-là ; 1 à 4 : par quartile, le maximum toujours au plus foncé. */
export type Heat = 'none' | 0 | 1 | 2 | 3 | 4

export function heatLevel(value: number | null, thresholds: number[]): Heat {
  if (value === null) return 'none'
  if (value <= 0 || thresholds.length === 0) return 0
  const [q1, q2, q3, max] = thresholds
  if (value >= max) return 4
  if (value <= q1) return 1
  if (value <= q2) return 2
  if (value <= q3) return 3
  return 4
}

/** Ce que lit un lecteur d'écran d'un mini-mois : son total, et ses jours sans données. */
export function monthSummary(month: string, days: MonthDay[], metric: MonthMetric | undefined, today: string): string {
  const name = `${monthName(month)} ${month.slice(0, 4)}`
  const past = days.filter((day) => day.date <= today)
  if (!metric || past.length === 0) return name
  const values = past.map((day) => day.values[metric.metric] ?? null)
  const known = values.filter((v): v is number => v !== null)
  if (known.length === 0) return `${name} : pas de données`
  const total = known.reduce((sum, v) => sum + v, 0)
  const missing = values.length - known.length
  const gaps = missing === 0 ? '' : `, ${missing} ${missing > 1 ? 'jours' : 'jour'} sans données`
  return `${name} : ${valueText(metric, total)}${gaps}`
}
