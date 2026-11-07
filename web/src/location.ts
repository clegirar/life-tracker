// Ce que la carte Déplacements dit d'un jour : la trace de l'iPhone (Overland) et les lieux où je suis
// resté, visites iOS et séjours détectés dans les points, avec leur durée, et celui où je suis encore ;
// sans nom de lieu (le géocodage inverse est un chantier à venir). Un jour hors de la période
// suivie (`covered` faux) est un jour sans données, jamais un jour sans déplacement.

import type { DayLocations, LocationPlace, SourcesHealth } from './api'
import { formatMinutes } from './music'
import { clock, dayWord, shortDate } from './status'
import { addDays, parisDate } from './time'

// Un remplacement de token crée une nouvelle source : `iphone-2`.
const isIphone = (name: string) => /^iphone\b/.test(name)

export type LatLng = [number, number]

/** Ce qu'on sait du jour : pas de données, suivi sans déplacement, ou une trace et des lieux. */
export type LocationKind = 'uncovered' | 'still' | 'moved'

export function locationKind(day: DayLocations): LocationKind {
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

/** La longueur de la trace du jour : chaque morceau, sans relier deux morceaux entre eux. */
export function traceLengthM(day: DayLocations): number {
  let total = 0
  for (const segment of day.segments) {
    for (let i = 1; i < segment.points.length; i++) total += distanceM(segment.points[i - 1], segment.points[i])
  }
  return total
}

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

/** La trace a au moins un morceau de deux points : il y a une longueur à dire. */
export const hasTrace = (day: DayLocations) => day.segments.some((segment) => segment.points.length > 1)

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

/** Ce que la carte dessine : les morceaux de trace, les points isolés et les épingles numérotées. */
export interface MapDrawing {
  lines: LatLng[][]
  dots: LatLng[]
  /** `current` : le lieu où je suis encore, mis en évidence. */
  pins: { at: LatLng; label: string; current: boolean }[]
}

/** En deçà, deux lieux sont le même sur la carte : la précision d'une visite iOS est de cet ordre. */
const SAME_PLACE_M = 50

export function mapDrawing(day: DayLocations, today: string): MapDrawing | null {
  const lines = day.segments.filter((segment) => segment.points.length > 1).map((segment) => segment.points)
  const dots = day.segments.filter((segment) => segment.points.length === 1).map((segment) => segment.points[0])
  // Deux lieux au même endroit (la maison, le soir et la nuit d'avant) : une seule épingle, « 1 · 4 ».
  const pins: MapDrawing['pins'] = []
  for (const row of placeRows(day, today)) {
    const same = pins.find((pin) => distanceM(pin.at, row.at) <= SAME_PLACE_M)
    if (same) {
      same.label += ` · ${row.number}`
      same.current ||= row.ongoing
    } else pins.push({ at: row.at, label: String(row.number), current: row.ongoing })
  }
  return lines.length + dots.length + pins.length === 0 ? null : { lines, dots, pins }
}

/** Tous les points à faire tenir dans la carte. */
export function drawingPoints(drawing: MapDrawing): LatLng[] {
  return [...drawing.lines.flat(), ...drawing.dots, ...drawing.pins.map((pin) => pin.at)]
}

/** Le texte de la carte pour un lecteur d'écran : la liste dessous en dit le détail. */
export function mapLabel(day: DayLocations): string {
  const places = day.places.length
  const parts = [places === 0 ? 'aucun lieu' : places === 1 ? '1 lieu' : `${places} lieux`]
  if (hasTrace(day)) parts.push(`${distanceText(traceLengthM(day))} de trace`)
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
