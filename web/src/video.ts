// Ce que la carte YouTube dit d'un jour. La durée est celle des vidéos, jamais le temps regardé : Google
// ne le donne pas. Une vidéo sans durée connue est comptée à part, jamais pour zéro, et une journée
// sans données ne se confond pas avec une journée sans vidéo.

import type { DayVideo, SourcesHealth, VideoWatch } from './api'
import { clockText } from './activity'
import { formatMinutes } from './music'
import { addDays, parisDate } from './time'

const WATCH_METRIC = 'video.watch'
// Un remplacement de token crée une nouvelle source : `youtube-2`.
const isYoutube = (name: string) => /^youtube\b/.test(name)

/** « 1 h 11 », « 45 min », « moins d'1 min » : la somme des durées connues. */
export function totalText(seconds: number | null): string {
  if (seconds === null) return '–'
  if (seconds > 0 && seconds < 60) return "moins d'1 min"
  return formatMinutes(Math.round(seconds / 60))
}

/** La page de la vidéo sur YouTube : un simple lien, sans intégration. */
export const videoUrl = (videoId: string) => `https://www.youtube.com/watch?v=${encodeURIComponent(videoId)}`

/** Un identifiant de vidéo YouTube : 11 caractères de l'alphabet base64 des URL. */
const VIDEO_ID = /^[A-Za-z0-9_-]{11}$/

/** La miniature 320 × 180 de `i.ytimg.com`, seul hôte d'images de YouTube que la CSP autorise ; `null` pour un identifiant douteux. */
export const thumbnailUrl = (videoId: string) => (VIDEO_ID.test(videoId) ? `https://i.ytimg.com/vi/${videoId}/mqdefault.jpg` : null)

/** Une ligne de la liste des vues. */
export interface WatchRow {
  key: string
  time: string
  /** `null` : vidéo supprimée ou privée, « Vidéo indisponible », sans lien ni chaîne. */
  title: string | null
  url: string | null
  /** La miniature ; `null` pour une vidéo indisponible ou un identifiant douteux. */
  thumbnail: string | null
  channel: string | null
  /** « 9:09 » ; `null` sans durée connue. */
  length: string | null
  /** Vue avant ce jour : ce n'est pas une découverte. */
  seenBefore: boolean
}

export function watchRows(day: DayVideo): WatchRow[] {
  const firstTime = new Set(day.discoveries.filter((d) => d.level === 'video').map((d) => d.key))
  return day.watches.map((watch) => ({
    key: `${watch.video_id}/${watch.occurred_at}`,
    time: watch.time,
    title: watch.title,
    url: watch.title === null ? null : videoUrl(watch.video_id),
    thumbnail: watch.title === null ? null : thumbnailUrl(watch.video_id),
    channel: watch.title === null ? null : watch.channel_name,
    length: watch.duration_s === null ? null : clockText(watch.duration_s),
    seenBefore: !firstTime.has(watch.video_id),
  }))
}

/** La chaîne la plus regardée du jour, à partir de deux vidéos ; la première vue en cas d'égalité. */
export function channelOfDay(watches: VideoWatch[]): { name: string; count: number } | null {
  const counts = new Map<string, { name: string; count: number }>()
  for (const watch of watches) {
    if (watch.channel_id === null || watch.channel_name === null) continue
    const entry = counts.get(watch.channel_id) ?? { name: watch.channel_name, count: 0 }
    entry.count++
    counts.set(watch.channel_id, entry)
  }
  let best: { name: string; count: number } | null = null
  for (const entry of counts.values()) if (entry.count >= 2 && (!best || entry.count > best.count)) best = entry
  return best
}

/** La source YouTube se tait : des vues de ce jour peuvent manquer. */
export const youtubeSilent = (health: SourcesHealth | null) =>
  health?.sources.some((source) => isYoutube(source.name) && source.silent) ?? false

/** Le texte d'une journée sans vidéo reçue. */
export function videoEmptyText(date: string, today: string, health: SourcesHealth | null): string {
  if (!health) return 'Aucune vidéo reçue pour ce jour.'
  if (!health.sources.some((source) => isYoutube(source.name))) return "YouTube n'est pas encore relié."
  const coverage = (health.metrics.find((m) => m.metric === WATCH_METRIC)?.coverage ?? []).filter((c) => isYoutube(c.source))
  if (coverage.length === 0) return "Pas encore de données YouTube : l'historique arrive avec le premier export."
  const day = (at: string) => parisDate(Date.parse(at))
  const first = coverage.map((c) => day(c.first_at)).sort()[0]
  const last = coverage.map((c) => day(c.last_at)).sort().at(-1)!
  if (date < first) return 'Pas de données YouTube à cette date.'
  // L'export n'a lieu qu'une fois par jour : après la dernière vue reçue, on ne sait pas encore.
  if (date > last) return "Pas encore de données pour ce jour : l'historique YouTube n'arrive qu'une fois par jour."
  if (date === today) return "Aucune vidéo aujourd'hui."
  if (date === addDays(today, -1)) return 'Aucune vidéo hier.'
  return 'Aucune vidéo ce jour-là.'
}
