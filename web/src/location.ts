// Ce que la carte Déplacements dit d'un jour : la trace de l'iPhone (Overland), les séances de la
// montre qui ont une trace (pendant une séance, la montre l'emporte) et les lieux où je suis resté,
// visites iOS et séjours détectés dans les points, avec leur durée, et celui où je suis encore ; sans
// nom de lieu (le géocodage inverse est un chantier à venir). Un jour hors de la période suivie
// (`covered` faux) et sans séance est un jour sans données, jamais un jour sans déplacement.

import type { DayLocations, LocationPlace, LocationWorkout, SourcesHealth } from './api'
import { workoutType } from './activity'
import { formatMinutes } from './music'
import { clock, dayWord, shortDate } from './status'
import { addDays, parisDate } from './time'

// Un remplacement de token crée une nouvelle source : `iphone-2`.
const isIphone = (name: string) => /^iphone\b/.test(name)

export type LatLng = [number, number]

/** Ce qu'on sait du jour : pas de données, suivi sans déplacement, ou une trace et des lieux. */
export type LocationKind = 'uncovered' | 'still' | 'moved'

export function locationKind(day: DayLocations): LocationKind {
  // Une séance de la montre se montre même un jour où l'iPhone n'a rien envoyé.
  if (day.workouts.length > 0) return 'moved'
  if (!day.covered) return 'uncovered'
  return day.segments.length === 0 && day.places.length === 0 ? 'still' : 'moved'
}

/** « 36 min », « 2 h 05 » ; moins d'une minute : « moins d'1 min ». */
export function durationText(seconds: number): string {
  const minutes = Math.round(seconds / 60)
  return minutes === 0 ? "moins d'1 min" : formatMinutes(minutes)
}

const EARTH_M = 6_371_000
const rad = (degrees: number) => (degrees * Math.PI) / 180

/** La distance à vol d'oiseau entre deux points, en mètres (haversine). */
export function distanceM([lat1, lon1]: LatLng, [lat2, lon2]: LatLng): number {
  const a = Math.sin(rad(lat2 - lat1) / 2) ** 2 + Math.cos(rad(lat1)) * Math.cos(rad(lat2)) * Math.sin(rad(lon2 - lon1) / 2) ** 2
  return 2 * EARTH_M * Math.asin(Math.min(1, Math.sqrt(a)))
}

/** La longueur d'une ligne de points. */
function lineLengthM(points: LatLng[]): number {
  let total = 0
  for (let i = 1; i < points.length; i++) total += distanceM(points[i - 1], points[i])
  return total
}

/** La longueur de la trace de l'iPhone : chaque morceau, sans relier deux morceaux entre eux. */
export function traceLengthM(day: DayLocations): number {
  return day.segments.reduce((total, segment) => total + lineLengthM(segment.points), 0)
}

/**
 * La distance d'une séance : le chiffre de Garmin, comme la carte Activité ; sans lui, la longueur
 * de sa trace.
 */
export const workoutLengthM = (workout: LocationWorkout) => workout.distance_m ?? lineLengthM(workout.points)

/** La distance du jour : la trace de l'iPhone, plus les séances. */
export const totalLengthM = (day: DayLocations) => day.workouts.reduce((total, workout) => total + workoutLengthM(workout), traceLengthM(day))

const KM = new Intl.NumberFormat('fr-FR', { minimumFractionDigits: 1, maximumFractionDigits: 1 })

/** « 850 m » sous le kilomètre, à 10 m près ; sinon « 12,4 km ». */
export function distanceText(meters: number): string {
  if (meters < 995) return `${Math.round(meters / 10) * 10} m`
  return `${KM.format(meters / 1000)} km`
}

/**
 * Les lieux du jour comme les compte le calendrier : par arrivée. Un lieu commencé avant (la nuit à
 * la maison) est dans la liste et sur la carte, et se dit à part : « et 1 depuis la veille ».
 */
export function placesCount(day: DayLocations): { arrived: number; earlier: string | null } {
  const earlier = day.places.filter((place) => place.arrived_before).length
  return { arrived: day.places.length - earlier, earlier: earlier === 0 ? null : `et ${earlier} depuis la veille` }
}

/** La trace de l'iPhone a au moins un morceau de deux points. */
const hasPhoneTrace = (day: DayLocations) => day.segments.some((segment) => segment.points.length > 1)

/** Il y a une longueur à dire : une trace de l'iPhone, ou une séance. */
export const hasTrace = (day: DayLocations) => hasPhoneTrace(day) || day.workouts.length > 0

/** « Course » donne « course », « Course sur tapis », « course sur tapis » : le nom dans une phrase. */
const workoutNoun = (type: string) => workoutType(type).toLowerCase()

/**
 * Sous la distance, la part des séances : « dont 8,2 km de course », « dont 8,2 km de course et
 * 12,0 km de vélo » ; une distance par type, dans l'ordre du jour. Rien sans séance.
 */
export function workoutsDetail(day: DayLocations): string | null {
  const byType = new Map<string, number>()
  for (const workout of day.workouts) {
    const noun = workoutNoun(workout.activity_type)
    byType.set(noun, (byType.get(noun) ?? 0) + workoutLengthM(workout))
  }
  const parts = [...byType].filter(([, meters]) => meters > 0).map(([noun, meters]) => `${distanceText(meters)} de ${noun}`)
  if (parts.length === 0) return null
  const last = parts.pop()!
  return `dont ${parts.length === 0 ? last : `${parts.join(', ')} et ${last}`}`
}

/** Une ligne de la liste des lieux, dans l'ordre d'arrivée ; `number` est celui de l'épingle. */
export interface PlaceRow {
  key: string
  number: number
  /** « 10:12 – 10:48 », ou « depuis 17:14 » pour le lieu où je suis encore. */
  range: string
  /** « Depuis la veille », « Jusqu'au lendemain » ; `null` pour un lieu tenu dans le jour. */
  note: string | null
  /** La durée entière du lieu, veille et lendemain compris ; « en cours » si j'y suis encore. */
  duration: string
  at: LatLng
  ongoing: boolean
  /** L'instant d'arrivée, en millisecondes. */
  arrival: number
}

/** « Depuis la veille », « Jusqu'au lendemain », « Depuis le 3 nov., jusqu'au lendemain ». */
function placeNote(place: LocationPlace, date: string, today: string): string | null {
  const dayOf = (instant: string) => parisDate(Date.parse(instant))
  const from = dayOf(place.arrival) === addDays(date, -1) ? 'la veille' : `le ${shortDate(dayOf(place.arrival), today)}`
  const departed = place.departed_after && place.departure !== null ? dayOf(place.departure) : null
  const to = departed === null ? null : departed === addDays(date, 1) ? 'lendemain' : shortDate(departed, today)
  if (place.arrived_before && to) return `Depuis ${from}, jusqu'au ${to}`
  if (place.arrived_before) return `Depuis ${from}`
  if (to) return `Jusqu'au ${to}`
  return null
}

const byArrival = (a: LocationPlace, b: LocationPlace) => Date.parse(a.arrival) - Date.parse(b.arrival)

/** Une séance dans la liste, à son heure parmi les lieux : sans numéro, les numéros sont ceux des lieux. */
export interface WorkoutRow {
  key: string
  /** « Course ». */
  label: string
  /** « 10:00 – 10:45 ». */
  range: string
  /** La distance, ou la durée d'une séance sans distance. */
  figure: string
  /** L'instant du début, en millisecondes. */
  start: number
}

export function workoutRows(day: DayLocations): WorkoutRow[] {
  return day.workouts.map((workout) => {
    const meters = workoutLengthM(workout)
    const start = Date.parse(workout.start)
    return {
      key: `workout-${workout.activity_id}`,
      label: workoutType(workout.activity_type),
      range: `${workout.start_time} – ${workout.end_time}`,
      figure: meters > 0 ? distanceText(meters) : durationText((Date.parse(workout.end) - start) / 1000),
      start,
    }
  })
}

/** Une ligne de la liste du jour : un lieu ou une séance. */
export type DayRow = { kind: 'place'; row: PlaceRow } | { kind: 'workout'; row: WorkoutRow }

/** Les lieux et les séances, dans l'ordre du temps : arrivée d'un lieu, début d'une séance. */
export function dayRows(day: DayLocations, today: string): DayRow[] {
  const rows: DayRow[] = [
    ...placeRows(day, today).map((row) => ({ kind: 'place' as const, row })),
    ...workoutRows(day).map((row) => ({ kind: 'workout' as const, row })),
  ]
  const at = (entry: DayRow) => (entry.kind === 'place' ? entry.row.arrival : entry.row.start)
  // Tri stable : à égalité, le lieu d'abord.
  return rows.sort((a, b) => at(a) - at(b))
}

/** Visites iOS et séjours détectés pareils : l'origine ne change ni la ligne ni l'épingle. */
export function placeRows(day: DayLocations, today: string): PlaceRow[] {
  return [...day.places].sort(byArrival).map((place, index) => ({
    key: `${place.origin}-${place.arrival}`,
    number: index + 1,
    range: place.ongoing || place.departure_time === null ? `depuis ${place.arrival_time}` : `${place.arrival_time} – ${place.departure_time}`,
    note: placeNote(place, day.date, today),
    duration: place.ongoing ? 'en cours' : durationText(place.duration_s),
    at: [place.lat, place.lon],
    ongoing: place.ongoing,
    arrival: Date.parse(place.arrival),
  }))
}

/**
 * Le haut de la carte quand j'y suis encore : « Ici depuis 17 h 14 », « depuis hier à 22 h 10 », et
 * le temps passé, « 1 h 12 ». Compté de minute d'horloge à minute d'horloge, comme on lit l'heure :
 * arrivé à 17 h 14, il est 18 h 26, cela fait 1 h 12.
 */
export function hereText(arrival: number, now: number): { since: string; elapsed: string } {
  const date = parisDate(arrival)
  const today = parisDate(now)
  const when = date === today ? clock(arrival) : `${dayWord(date, today)} à ${clock(arrival)}`
  const minutes = Math.max(0, Math.floor(now / 60_000) - Math.floor(arrival / 60_000))
  return { since: `Ici depuis ${when}`, elapsed: minutes === 0 ? "moins d'1 min" : formatMinutes(minutes) }
}

/** Ce que la carte dessine : les morceaux de trace, les séances, les points isolés et les épingles numérotées. */
export interface MapDrawing {
  lines: LatLng[][]
  /** Les traces des séances, dans la couleur des séances. */
  workouts: LatLng[][]
  dots: LatLng[]
  /** `current` : le lieu où je suis encore, mis en évidence. */
  pins: { at: LatLng; label: string; current: boolean }[]
}

/** En deçà, deux lieux sont le même sur la carte : la précision d'une visite iOS est de cet ordre. */
const SAME_PLACE_M = 50

export function mapDrawing(day: DayLocations, today: string): MapDrawing | null {
  const lines = day.segments.filter((segment) => segment.points.length > 1).map((segment) => segment.points)
  const dots = day.segments.filter((segment) => segment.points.length === 1).map((segment) => segment.points[0])
  const workouts = day.workouts.filter((workout) => workout.points.length > 1).map((workout) => workout.points)
  // Deux lieux au même endroit (la maison, le soir et la nuit d'avant) : une seule épingle, « 1 · 4 ».
  const pins: MapDrawing['pins'] = []
  for (const row of placeRows(day, today)) {
    const same = pins.find((pin) => distanceM(pin.at, row.at) <= SAME_PLACE_M)
    if (same) {
      same.label += ` · ${row.number}`
      same.current ||= row.ongoing
    } else pins.push({ at: row.at, label: String(row.number), current: row.ongoing })
  }
  return lines.length + workouts.length + dots.length + pins.length === 0 ? null : { lines, workouts, dots, pins }
}

/** Tous les points à faire tenir dans la carte. */
export function drawingPoints(drawing: MapDrawing): LatLng[] {
  return [...drawing.lines.flat(), ...drawing.workouts.flat(), ...drawing.dots, ...drawing.pins.map((pin) => pin.at)]
}

/** Le texte de la carte pour un lecteur d'écran : la liste dessous en dit le détail. */
export function mapLabel(day: DayLocations): string {
  const places = day.places.length
  const parts = [places === 0 ? 'aucun lieu' : places === 1 ? '1 lieu' : `${places} lieux`]
  if (hasPhoneTrace(day)) parts.push(`${distanceText(traceLengthM(day))} de trace`)
  const workouts = day.workouts.length
  if (workouts > 0) parts.push(workouts === 1 ? '1 séance' : `${workouts} séances`)
  const current = placeRows(day, day.date).find((row) => row.ongoing)
  if (current) parts.push(`lieu ${current.number} en cours`)
  return `Carte des déplacements : ${parts.join(', ')}`
}

/** La source de l'iPhone se tait : des déplacements de ce jour peuvent manquer. */
export const iphoneSilent = (health: SourcesHealth | null) =>
  health?.sources.some((source) => isIphone(source.name) && source.silent) ?? false

/** Un jour suivi, sans trace ni lieu. */
export function stillText(date: string, today: string): string {
  if (date === today) return "Aucun déplacement enregistré pour l'instant aujourd'hui."
  if (date === addDays(today, -1)) return 'Aucun déplacement enregistré hier.'
  return 'Aucun déplacement enregistré ce jour-là.'
}

/**
 * Un jour sans données. Overland garde ses points hors réseau et les envoie plus tard : après son
 * dernier envoi, on ne sait pas encore. Avant la première donnée, il n'y a simplement pas de données.
 */
export function uncoveredText(date: string, today: string, health: SourcesHealth | null): string {
  if (!health) return 'Aucune position reçue pour ce jour.'
  const sources = health.sources.filter((source) => isIphone(source.name))
  if (sources.length === 0) return "La position de l'iPhone n'est pas encore reliée."
  const seen = sources
    .map((source) => source.last_seen_at)
    .filter((at): at is string => at !== null)
    .map((at) => parisDate(Date.parse(at)))
    .sort()
    .at(-1)
  if (date === today && (seen === undefined || seen <= today)) return "Pas encore de position reçue aujourd'hui."
  if (seen === undefined || date > seen) return "Pas encore de données pour ce jour : l'iPhone n'a rien envoyé depuis."
  if (date === seen) return 'Pas encore de position reçue pour ce jour.'
  return 'Pas de données de position à cette date.'
}
