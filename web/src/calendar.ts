// La vue du mois : la grille, et ce que chaque case dit de sa journée.
// Une case sans chiffre ou au numéro grisé n'a pas de données ; un 0 ou une case sans point est une
// journée sans activité. Jamais l'un pour l'autre.

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

const SOURCE_NAMES: Record<string, string> = { lastfm: 'Last.fm', garmin: 'Garmin' }

/** Le nom d'une source tel qu'on le dit : `lastfm-2` reste Last.fm. */
export function sourceName(name: string): string {
  const base = name.replace(/-\d+$/, '')
  return SOURCE_NAMES[base] ?? name
}

function plural(count: number, one: string, many: string): string {
  return `${count} ${count > 1 ? many : one}`
}

const NUMBER = new Intl.NumberFormat('fr-FR', { maximumFractionDigits: 0 })

/** « 7 h 12 », « 45 min » : des minutes dites comme une durée. */
export function hoursText(minutes: number): string {
  const total = Math.round(minutes)
  if (total < 60) return `${total} min`
  return `${Math.floor(total / 60)} h ${String(total % 60).padStart(2, '0')}`
}

/** Le nom d'une métrique dans une phrase : la majuscule initiale tombe, celle d'un nom propre reste. */
export const lowerLabel = (label: string) => label.charAt(0).toLowerCase() + label.slice(1)

/** Les unités qui se disent après le nombre, sans accord : « 8 412 pas », « 2 350 kcal ». */
const PLAIN_UNITS = new Set(['pas', 'kcal'])

/**
 * « 3 cafés », « 1 séance », « 8 412 pas », « 2 350 kcal », « 7 h 12 », « pas de données » : l'unité
 * du registre dit comment, et le nom de la métrique sinon.
 */
export function valueText(metric: MonthMetric, value: number | null): string {
  const label = lowerLabel(metric.label)
  if (value === null) return `${label} : pas de données`
  if (metric.unit === 'min') return hoursText(value)
  if (metric.unit && PLAIN_UNITS.has(metric.unit)) return `${NUMBER.format(value)} ${metric.unit}`
  // Le premier mot porte le pluriel : « 1 vidéo YouTube », « 3 vidéos YouTube ».
  const [head, ...rest] = label.split(' ')
  const one = [head.endsWith('s') ? head.slice(0, -1) : head, ...rest].join(' ')
  return plural(Math.round(value), one, label)
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
    notes.push(`${metric ? lowerLabel(metric.label) : name} à vérifier`)
  }
  return notes
}

/** Ce que lit un lecteur d'écran sur une case : chaque métrique par son nom, jamais par sa couleur. */
export function cellLabel(day: MonthDay, metrics: MonthMetric[], today: string): string {
  const known = metrics.some((metric) => (day.values[metric.metric] ?? null) !== null)
  const parts =
    metrics.length === 0 ? [] : metrics.length > 1 && !known ? ['pas de données'] : metrics.map((metric) => valueText(metric, day.values[metric.metric] ?? null))
  const said = [...parts, ...dayNotes(day, metrics)].join(', ')
  // Aucune métrique choisie et rien à signaler : la date seule.
  return said ? `${longDate(day.date, today)} : ${said}` : longDate(day.date, today)
}

/** Le chiffre d'une case : 1 234 devient « 1,2 k » pour tenir dans la largeur d'un jour. */
export function compact(value: number): string {
  if (value < 1000) return String(Math.round(value))
  return `${(value / 1000).toFixed(1).replace('.', ',').replace(',0', '')} k`
}

/** Le chiffre d'une case selon l'unité : des minutes en heures (« 7,2 h »), le reste en `compact`. */
export function cellText(metric: MonthMetric | undefined, value: number): string {
  if (metric?.unit === 'min' && value >= 60) return `${(value / 60).toFixed(1).replace('.', ',').replace(',0', '')} h`
  return compact(value)
}

/** Le compteur global : jamais une absence, même à zéro. */
export function errorCountText(errors: number): string {
  if (errors === 0) return 'Aucun enregistrement en erreur'
  return `${plural(errors, 'enregistrement', 'enregistrements')} en erreur`
}
