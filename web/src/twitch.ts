// Ce que la carte Twitch dit d'un jour. Le temps est celui de la lecture, mesuré par l'extension
// Firefox : le total est l'union des segments (deux streams à la fois comptent une fois), chaque chaîne
// garde son temps entier. Une journée sans données ne se confond jamais avec une journée sans Twitch.

import type { DayStream, SourcesHealth } from './api'
import { formatMinutes } from './music'
import { addDays, parisDate } from './time'

// Un remplacement de token crée une nouvelle source : `twitch-2`.
const isTwitch = (name: string) => /^twitch\b/.test(name)

/** Rien n'a été reçu ce jour-là : la carte dit pourquoi, sans chiffre. */
export const watchedTwitch = (day: DayStream) => day.segments > 0

/** « 2 h 05 », « 45 min » ; des segments reçus mais moins d'une minute en tout : « moins d'1 min ». */
export function streamMinutesText(minutes: number): string {
  return minutes === 0 ? "moins d'1 min" : formatMinutes(minutes)
}

/** « dont 25 min à deux streams » : le temps compté pour deux chaînes à la fois, à partir d'une minute. */
export function overlapText(day: DayStream): string | null {
  const overlap = day.summed_minutes - day.minutes
  return overlap >= 1 ? `dont ${formatMinutes(overlap)} à deux streams` : null
}

/** Un login Twitch : 1 à 25 lettres minuscules, chiffres ou soulignés. Sinon, pas de lien. */
const LOGIN = /^[a-z0-9_]{1,25}$/

/** La chaîne sur Twitch : un simple lien, sans intégration ni image. */
export const channelUrl = (login: string) => (LOGIN.test(login) ? `https://www.twitch.tv/${login}` : null)

const CONTENT_WORDS: Record<string, string> = { live: 'Live', vod: 'Rediffusion', clip: 'Clip' }

/** Une ligne de la liste des chaînes. */
export interface ChannelRow {
  key: string
  /** `null` : la chaîne inconnue, sans lien. */
  name: string | null
  url: string | null
  /** « Live · Rediffusion » ; vide pour la chaîne inconnue. */
  contents: string[]
  /** « 20:30 – 23:15 » ; `null` pour la chaîne inconnue. */
  span: string | null
  minutes: string
}

/** Les chaînes par temps, dans l'ordre du serveur, puis le temps sans chaîne. */
export function channelRows(day: DayStream): ChannelRow[] {
  const rows: ChannelRow[] = day.channels.map((channel) => ({
    key: channel.key,
    name: channel.name,
    url: channelUrl(channel.key),
    contents: channel.contents.map((content) => CONTENT_WORDS[content] ?? content),
    span: `${channel.first_time} – ${channel.last_time}`,
    minutes: streamMinutesText(channel.minutes),
  }))
  if (day.unknown_minutes > 0) {
    rows.push({ key: '', name: null, url: null, contents: [], span: null, minutes: streamMinutesText(day.unknown_minutes) })
  }
  return rows
}

/** Les chaînes regardées pour la première fois ce jour-là, dans l'ordre du serveur. */
export const newStreamChannels = (day: DayStream) => day.discoveries.filter((d) => d.level === 'channel')

/** La source Twitch se tait : des streams de ce jour peuvent manquer. */
export const twitchSilent = (health: SourcesHealth | null) =>
  health?.sources.some((source) => isTwitch(source.name) && source.silent) ?? false

/**
 * Le texte d'une journée sans segment reçu. `value` est la case du mois pour `stream.watch` : `null`
 * avant la première donnée, `undefined` tant que le mois n'est pas lu. L'extension envoie au fil de la
 * lecture et garde hors ligne ce qu'elle n'a pas pu remettre : après son dernier signal, on ne sait pas encore.
 */
export function twitchEmptyText(date: string, today: string, health: SourcesHealth | null, value: number | null | undefined): string {
  if (!health || value === undefined) return 'Aucun stream reçu pour ce jour.'
  const sources = health.sources.filter((source) => isTwitch(source.name))
  if (sources.length === 0) return "Twitch n'est pas encore relié."
  if (value === null) return 'Pas de données Twitch à cette date.'
  const seen = sources
    .map((source) => source.last_seen_at)
    .filter((at): at is string => at !== null)
    .map((at) => parisDate(Date.parse(at)))
    .sort()
    .at(-1)
  if (seen === undefined || date > seen) return "Pas encore de données pour ce jour : Firefox n'a pas donné signe de vie depuis."
  if (date === today) return "Pas de Twitch aujourd'hui."
  if (date === addDays(today, -1)) return 'Pas de Twitch hier.'
  return 'Pas de Twitch ce jour-là.'
}
