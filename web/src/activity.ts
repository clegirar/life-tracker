// Ce que la carte Activité dit des données de Garmin : des chiffres à la française, et jamais un zéro
// à la place d'une journée sans données.

import type { DayActivity, SleepDetails, WorkoutDetails } from './api'
import { hoursText } from './calendar'
import { parisTime } from './time'

const INTEGER = new Intl.NumberFormat('fr-FR', { maximumFractionDigits: 0 })
const KM = new Intl.NumberFormat('fr-FR', { minimumFractionDigits: 2, maximumFractionDigits: 2 })

/** Le type de Garmin en français ; un type inconnu garde sa clé, lisible à défaut d'être traduit. */
const TYPES: Record<string, string> = {
  running: 'Course',
  trail_running: 'Trail',
  treadmill_running: 'Course sur tapis',
  walking: 'Marche',
  hiking: 'Randonnée',
  cycling: 'Vélo',
  swimming: 'Natation',
  strength_training: 'Renforcement',
}

export const workoutType = (type: string) => TYPES[type] ?? type

const RUNS = new Set(['running', 'trail_running', 'treadmill_running'])

export const integerText = (value: number) => INTEGER.format(value)

/** « 6,12 km ». */
export const kmText = (meters: number) => `${KM.format(meters / 1000)} km`

/** « 32:05 », « 1:02:05 » : une durée de séance, comme sur un chrono. */
export function clockText(seconds: number): string {
  const total = Math.round(seconds)
  const h = Math.floor(total / 3600)
  const m = Math.floor((total % 3600) / 60)
  const s = String(total % 60).padStart(2, '0')
  return h > 0 ? `${h}:${String(m).padStart(2, '0')}:${s}` : `${m}:${s}`
}

/** « 5:59 /km » pour une course, « 24,3 km/h » pour le reste ; rien sans vitesse. */
export function paceText(details: WorkoutDetails): string | null {
  const speed = details.avg_speed_mps
  if (!speed || speed <= 0) return null
  if (RUNS.has(details.type)) {
    const perKm = Math.round(1000 / speed)
    return `${Math.floor(perKm / 60)}:${String(perKm % 60).padStart(2, '0')} /km`
  }
  return `${(speed * 3.6).toFixed(1).replace('.', ',')} km/h`
}

/** Les chiffres d'une séance, dans l'ordre de Garmin Connect : distance, durée, allure, fréquence cardiaque. */
export function workoutFigures(details: WorkoutDetails): { label: string; value: string }[] {
  const figures: { label: string; value: string }[] = []
  if (details.distance_m) figures.push({ label: 'Distance', value: kmText(details.distance_m) })
  const duration = details.moving_s ?? details.duration_s
  if (duration) figures.push({ label: 'Durée', value: clockText(duration) })
  const pace = paceText(details)
  if (pace) figures.push({ label: RUNS.has(details.type) ? 'Allure' : 'Vitesse', value: pace })
  if (details.avg_hr) figures.push({ label: 'FC moyenne', value: `${integerText(details.avg_hr)} bpm` })
  if (details.max_hr) figures.push({ label: 'FC max', value: `${integerText(details.max_hr)} bpm` })
  return figures
}

export interface SleepStage {
  key: 'deep' | 'light' | 'rem' | 'awake'
  label: string
  minutes: number
}

/** Les phases connues de la nuit, dans l'ordre du sommeil le plus profond à l'éveil. */
export function sleepStages(details: SleepDetails): SleepStage[] {
  const stages: [SleepStage['key'], string, number | null][] = [
    ['deep', 'Profond', details.deep_seconds],
    ['light', 'Léger', details.light_seconds],
    ['rem', 'Paradoxal', details.rem_seconds],
    ['awake', 'Éveil', details.awake_seconds],
  ]
  return stages
    .filter(([, , seconds]) => seconds !== null && seconds > 0)
    .map(([key, label, seconds]) => ({ key, label, minutes: Math.round((seconds ?? 0) / 60) }))
}

/** « 22:41 – 06:05 », heure de Paris. */
export function sleepSpan(details: SleepDetails): string | null {
  if (!details.start || !details.end) return null
  return `${parisTime(Date.parse(details.start))} – ${parisTime(Date.parse(details.end))}`
}

export const sleepText = (minutes: number) => hoursText(minutes)

/** « FC la plus basse 48 bpm · moyenne 55 bpm » ; rien sans fréquence cardiaque. */
export function sleepHeartRate(details: SleepDetails): string | null {
  const parts = [
    details.lowest_hr != null && `FC la plus basse ${integerText(details.lowest_hr)} bpm`,
    details.avg_hr != null && `${details.lowest_hr != null ? 'moyenne' : 'FC moyenne'} ${integerText(details.avg_hr)} bpm`,
  ].filter(Boolean)
  return parts.length > 0 ? parts.join(' · ') : null
}

/** Aucune donnée Garmin ce jour-là : ni pas, ni sommeil, ni calories, ni séance. */
export function isEmpty(day: DayActivity): boolean {
  return day.steps === null && day.calories === null && day.sleep === null && day.workouts.length === 0
}
