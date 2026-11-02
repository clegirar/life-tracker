// La vue du mois : la grille, et ce que chaque case dit de sa journée.
// Une case sans chiffre n'a pas de données ; un 0 est une journée sans activité. Jamais l'un pour l'autre.

import type { MonthDay, MonthMetric } from './api'
import { longDate } from './time'

/** `AAAA-MM` d'une date `AAAA-MM-JJ`. */
export const monthOf = (date: string) => date.slice(0, 7)

export function addMonths(month: string, months: number): string {
  const [year, m] = month.split('-').map(Number)
  const date = new Date(Date.UTC(year, m - 1 + months, 1))
  return date.toISOString().slice(0, 7)
}

const capitalize = (text: string) => text.charAt(0).toUpperCase() + text.slice(1)

/** « Novembre ». */
export function monthName(month: string): string {
  return capitalize(
    new Intl.DateTimeFormat('fr-FR', { timeZone: 'UTC', month: 'long' }).format(new Date(`${month}-15T12:00:00Z`)),
  )
}

/** Les semaines du mois, lundi d'abord ; `null` pour les cases d'un autre mois. */
export function monthGrid(month: string): (string | null)[][] {
  const [year, m] = month.split('-').map(Number)
  const first = new Date(Date.UTC(year, m - 1, 1))
  const length = new Date(Date.UTC(year, m, 0)).getUTCDate()
  const lead = (first.getUTCDay() + 6) % 7
  const cells: (string | null)[] = Array.from({ length: lead }, () => null)
  for (let day = 1; day <= length; day++) cells.push(`${month}-${String(day).padStart(2, '0')}`)
  while (cells.length % 7 !== 0) cells.push(null)
  const weeks: (string | null)[][] = []
  for (let i = 0; i < cells.length; i += 7) weeks.push(cells.slice(i, i + 7))
  return weeks
}

export const WEEKDAYS: [string, string][] = [
  ['L', 'lundi'],
  ['M', 'mardi'],
  ['M', 'mercredi'],
  ['J', 'jeudi'],
  ['V', 'vendredi'],
  ['S', 'samedi'],
  ['D', 'dimanche'],
]

/** Une seule marque par case, la plus grave d'abord ; la légende et le libellé disent le reste. */
export type Marker = 'incomplete' | 'pending' | 'check'

export function marker(day: MonthDay): Marker | null {
  if (day.errors > 0) return 'incomplete'
  if (day.pending.length > 0) return 'pending'
  if (day.to_check.length > 0) return 'check'
  return null
}

export const MARKER_LABELS: Record<Marker, string> = {
  incomplete: 'Incomplète : erreurs',
  pending: 'En attente d’une source',
  check: 'À vérifier',
}

const SOURCE_NAMES: Record<string, string> = { lastfm: 'Last.fm' }

/** Le nom d'une source tel qu'on le dit : `lastfm-2` reste Last.fm. */
export function sourceName(name: string): string {
  const base = name.replace(/-\d+$/, '')
  return SOURCE_NAMES[base] ?? name
}

function plural(count: number, one: string, many: string): string {
  return `${count} ${count > 1 ? many : one}`
}

/** « 3 cafés », « 1 écoute », « pas de données » : le nom de la métrique vient du registre. */
export function valueText(metric: MonthMetric, value: number | null): string {
  const label = metric.label.toLowerCase()
  if (value === null) return `${label} : pas de données`
  const one = label.endsWith('s') ? label.slice(0, -1) : label
  return plural(value, one, label)
}

/** Les phrases de complétude d'une journée, pour la page du jour et le libellé de la case. */
export function dayNotes(day: MonthDay, metrics: MonthMetric[]): string[] {
  const notes: string[] = []
  if (day.errors > 0) {
    notes.push(`journée incomplète, ${plural(day.errors, 'enregistrement', 'enregistrements')} en erreur`)
  }
  for (const source of day.pending) notes.push(`en attente de ${sourceName(source)}`)
  for (const name of day.to_check) {
    const metric = metrics.find((m) => m.metric === name)
    notes.push(`${metric ? metric.label.toLowerCase() : name} à vérifier`)
  }
  return notes
}

/** Ce que lit un lecteur d'écran sur une case. */
export function cellLabel(day: MonthDay, metrics: MonthMetric[], today: string): string {
  const parts = metrics.map((metric) => valueText(metric, day.values[metric.metric] ?? null))
  return [longDate(day.date, today), [...parts, ...dayNotes(day, metrics)].join(', ')].join(' : ')
}

/** Le chiffre d'une case : 1 234 devient « 1,2 k » pour tenir dans la largeur d'un jour. */
export function compact(value: number): string {
  if (value < 1000) return String(Math.round(value))
  return `${(value / 1000).toFixed(1).replace('.', ',').replace(',0', '')} k`
}

/** Le compteur global : jamais une absence, même à zéro. */
export function errorCountText(errors: number): string {
  if (errors === 0) return 'Aucun enregistrement en erreur'
  return `${plural(errors, 'enregistrement', 'enregistrements')} en erreur`
}
