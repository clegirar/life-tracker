// Ce que la page dit de la musique. Une journée sans données ne se confond jamais avec une journée
// sans écoute : le texte dépend de ce que l'on sait de la source.

import type { MetricHealth, SourceHealth, SourcesHealth } from './api'
import { addDays, parisDate } from './time'

const LISTEN_METRIC = 'music.listen'
// Un remplacement de token crée une nouvelle source : `lastfm-2`.
const isLastfm = (source: SourceHealth) => /^lastfm\b/.test(source.name)

export function formatMinutes(minutes: number | null): string {
  if (minutes === null) return '–'
  if (minutes < 60) return `${minutes} min`
  return `${Math.floor(minutes / 60)} h ${String(minutes % 60).padStart(2, '0')}`
}

export function plural(count: number, word: string): string {
  return `${count} ${word}${count > 1 ? 's' : ''}`
}

const listenMetric = (health: SourcesHealth): MetricHealth | undefined =>
  health.metrics.find((metric) => metric.metric === LISTEN_METRIC)

export interface Alert {
  title: string
  text: string
}

/** La panne d'abord : si le connecteur se tait, l'absence de nouvelles écoutes n'apprend rien de plus. */
export function musicAlert(health: SourcesHealth | null): Alert | null {
  if (!health) return null
  const silent = health.sources.find((source) => isLastfm(source) && source.silent)
  if (silent) {
    const hours = Math.round(silent.silence_threshold_seconds / 3600)
    return {
      title: 'Last.fm ne répond plus.',
      text: `Aucun contact réussi depuis plus de ${hours} h : des écoutes peuvent manquer.`,
    }
  }
  const metric = listenMetric(health)
  if (metric?.stale) {
    const days = Math.round(metric.freshness_threshold_seconds / 86_400)
    return {
      title: 'Musique à vérifier.',
      text: `Aucune nouvelle écoute depuis ${days} jours : pause musicale, ou scrobbler arrêté ?`,
    }
  }
  return null
}

/** Le texte d'une journée sans écoute reçue. */
export function musicEmptyText(date: string, today: string, health: SourcesHealth | null): string {
  if (!health) return 'Aucune écoute reçue pour ce jour.'
  const coverage = listenMetric(health)?.coverage ?? []
  const day = (at: string) => parisDate(Date.parse(at))
  // Un historique importé couvre sa période, quoi que devienne Last.fm ensuite.
  const inHistory = coverage.some(
    (c) => !/^lastfm\b/.test(c.source) && day(c.first_at) <= date && date <= day(c.last_at),
  )
  if (inHistory) return date === addDays(today, -1) ? 'Aucune écoute hier.' : 'Aucune écoute ce jour-là.'
  const sources = health.sources.filter(isLastfm)
  if (sources.length === 0) return "Last.fm n'est pas encore relié."
  if (sources.some((source) => source.silent)) {
    return 'Pas de données : Last.fm ne répond plus, cette journée peut être incomplète.'
  }
  // Avant Last.fm et hors de tout historique, on ne sait pas : le trou entre deux sources n'est pas un silence.
  const first = coverage.find((c) => /^lastfm\b/.test(c.source))?.first_at
  if (!first || date < day(first)) {
    if (date !== today) return 'Pas de données musicales à cette date.'
  }
  if (date === today) return "Aucune écoute aujourd'hui."
  if (date === addDays(today, -1)) return 'Aucune écoute hier.'
  return 'Aucune écoute ce jour-là.'
}
