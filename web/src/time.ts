// Tout se raisonne en heure de Paris, quel que soit le fuseau de l'appareil.
// Une date est une chaîne AAAA-MM-JJ, une heure HH:MM, un instant des millisecondes.

const ZONE = 'Europe/Paris'

const partsFormat = new Intl.DateTimeFormat('en-CA', {
  timeZone: ZONE,
  hourCycle: 'h23',
  year: 'numeric',
  month: '2-digit',
  day: '2-digit',
  hour: '2-digit',
  minute: '2-digit',
  second: '2-digit',
})

function parisParts(instant: number) {
  const parts: Record<string, string> = {}
  for (const { type, value } of partsFormat.formatToParts(instant)) parts[type] = value
  return parts
}

export function parisDate(instant: number): string {
  const { year, month, day } = parisParts(instant)
  return `${year}-${month}-${day}`
}

export function parisTime(instant: number): string {
  const { hour, minute } = parisParts(instant)
  return `${hour}:${minute}`
}

function offsetMinutes(instant: number): number {
  const { year, month, day, hour, minute, second } = parisParts(instant)
  const asUtc = Date.UTC(+year, +month - 1, +day, +hour, +minute, +second)
  return Math.round((asUtc - Math.floor(instant / 1000) * 1000) / 60_000)
}

/** RFC 3339 avec le décalage de Paris à cet instant, jamais `Z` (qui voudrait dire « inconnu »). */
export function toRfc3339(instant: number): string {
  const { year, month, day, hour, minute, second } = parisParts(instant)
  const offset = offsetMinutes(instant)
  const sign = offset < 0 ? '-' : '+'
  const hours = String(Math.floor(Math.abs(offset) / 60)).padStart(2, '0')
  const minutes = String(Math.abs(offset) % 60).padStart(2, '0')
  return `${year}-${month}-${day}T${hour}:${minute}:${second}${sign}${hours}:${minutes}`
}

/**
 * L'instant d'une heure de Paris un jour donné. Deux cas au changement d'heure :
 * - l'heure existe deux fois (02:30 le 25 oct. 2026) : c'est le premier passage, en heure d'été ;
 * - l'heure n'existe pas (02:30 le 29 mars 2026) : `null`, l'interface la refuse.
 */
export function parisInstant(date: string, time: string): number | null {
  const [year, month, day] = date.split('-').map(Number)
  const [hour, minute] = time.split(':').map(Number)
  const wall = Date.UTC(year, month - 1, day, hour, minute)
  // Les décalages possibles sont ceux de la veille et du lendemain.
  const offsets = [offsetMinutes(wall - 86_400_000), offsetMinutes(wall + 86_400_000)]
  const matches = offsets
    .map((offset) => wall - offset * 60_000)
    .filter((instant) => parisDate(instant) === date && parisTime(instant) === time)
  return matches.length > 0 ? Math.min(...matches) : null
}

export function addDays(date: string, days: number): string {
  const [year, month, day] = date.split('-').map(Number)
  return new Date(Date.UTC(year, month - 1, day + days)).toISOString().slice(0, 10)
}

function capitalize(text: string): string {
  return text.charAt(0).toUpperCase() + text.slice(1)
}

function formatDate(date: string, options: Intl.DateTimeFormatOptions): string {
  return new Intl.DateTimeFormat('fr-FR', { timeZone: 'UTC', ...options }).format(
    new Date(`${date}T12:00:00Z`),
  )
}

/** « Samedi 31 octobre », avec l'année si ce n'est pas celle d'aujourd'hui. */
export function longDate(date: string, today: string): string {
  const year = date.slice(0, 4) === today.slice(0, 4) ? undefined : 'numeric'
  return capitalize(
    formatDate(date, { weekday: 'long', day: 'numeric', month: 'long', year }),
  )
}

/** « Aujourd'hui », « Hier », sinon le jour de la semaine. */
export function dayTitle(date: string, today: string): string {
  if (date === today) return "Aujourd'hui"
  if (date === addDays(today, -1)) return 'Hier'
  return capitalize(formatDate(date, { weekday: 'long' }))
}

export function emptyLabel(date: string, today: string): string {
  if (date === today) return "Aucun café noté aujourd'hui."
  if (date === addDays(today, -1)) return 'Aucun café noté hier.'
  return 'Aucun café noté ce jour-là.'
}
