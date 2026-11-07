// Ce que la carte Steam dit d'un jour. Steam ne donne que des compteurs cumulés : le temps d'un jour
// vient de l'écart entre deux relevés du collecteur, placé à la fin de la partie. Les succès arrivent
// avec leur vraie date. Une journée sans données ne se confond jamais avec une journée sans jeu.

import type { DayGame, DayGames, GameDiscovery, SourcesHealth } from './api'
import { formatMinutes } from './music'
import { addDays, parisDate } from './time'

// Un remplacement de token crée une nouvelle source : `steam-2`.
const isSteam = (name: string) => /^steam\b/.test(name)

const NUMBER = new Intl.NumberFormat('fr-FR', { maximumFractionDigits: 0 })

/** Rien n'a été reçu ce jour-là, ni partie ni succès : la carte dit pourquoi, sans chiffre. */
export const playedSteam = (day: DayGames) => day.segments > 0 || day.achievements > 0

/** « 2 h 05 », « 40 min » ; une partie reçue de moins d'une minute : « moins d'1 min ». */
export function playMinutesText(minutes: number): string {
  return minutes === 0 ? "moins d'1 min" : formatMinutes(minutes)
}

/**
 * Le total de Steam pour un jeu : « 45 min au total », « 2 h 40 au total », puis en heures entières
 * dès 10 h, comme Steam les arrondit (« 160 h au total »).
 */
export function totalText(minutes: number | null): string | null {
  if (minutes === null || minutes <= 0) return null
  if (minutes < 600) return `${formatMinutes(minutes)} au total`
  return `${NUMBER.format(Math.round(minutes / 60))} h au total`
}

/** Un appid Steam : un entier positif. Sinon, pas de lien. */
const APPID = /^[1-9][0-9]{0,9}$/

/** La page du jeu sur le magasin Steam : un simple lien, sans image. */
export const storeUrl = (appid: string) => (APPID.test(appid) ? `https://store.steampowered.com/app/${appid}/` : null)

/** Une ligne de la liste des jeux. */
export interface GameRow {
  key: string
  name: string
  url: string | null
  /** « 21:15 – 21:55 · 2 h 40 au total » ; vide si rien n'est connu. */
  detail: string
  /** `null` : un jeu qui n'a qu'un succès ce jour-là, sans partie reçue. */
  minutes: string | null
  achievements: { key: string; name: string; time: string }[]
}

function gameRow(game: DayGame): GameRow {
  const played = game.first_time !== null && game.last_time !== null
  const span = played ? `${game.first_time} – ${game.last_time}` : null
  return {
    key: game.key,
    name: game.name,
    url: storeUrl(game.key),
    detail: [span, totalText(game.total_minutes)].filter(Boolean).join(' · '),
    minutes: played ? playMinutesText(game.minutes) : null,
    achievements: game.achievements,
  }
}

/** Les jeux dans l'ordre du serveur : joués par temps, puis ceux qui n'ont qu'un succès. */
export const gameRows = (day: DayGames): GameRow[] => day.games.map(gameRow)

/** Une ligne des découvertes : un jeu joué pour la première fois ce jour-là. */
export interface DiscoveryRow {
  key: string
  name: string
  url: string | null
}

/**
 * Les jeux découverts ce jour-là, dans l'ordre du serveur (par première heure). Vide : la carte ne
 * montre pas la section, un jour sans nouveau jeu est la règle.
 */
export const discoveryRows = (day: DayGames): DiscoveryRow[] =>
  day.discoveries.map((found: GameDiscovery) => ({ key: found.key, name: found.name, url: storeUrl(found.key) }))

/** La source Steam se tait : des parties de ce jour peuvent manquer. */
export const steamSilent = (health: SourcesHealth | null) =>
  health?.sources.some((source) => isSteam(source.name) && source.silent) ?? false

/**
 * Le texte d'une journée sans partie ni succès reçus. `value` est la case du mois pour `game.play` :
 * `null` avant le premier relevé, `undefined` tant que le mois n'est pas lu. Le collecteur relève les
 * compteurs tous les quarts d'heure : après son dernier passage, on ne sait pas encore.
 */
export function steamEmptyText(date: string, today: string, health: SourcesHealth | null, value: number | null | undefined): string {
  if (!health || value === undefined) return 'Aucune partie reçue pour ce jour.'
  const sources = health.sources.filter((source) => isSteam(source.name))
  if (sources.length === 0) return "Steam n'est pas encore relié."
  if (value === null) return 'Pas de données Steam à cette date.'
  const seen = sources
    .map((source) => source.last_seen_at)
    .filter((at): at is string => at !== null)
    .map((at) => parisDate(Date.parse(at)))
    .sort()
    .at(-1)
  if (seen === undefined || date > seen) return "Pas encore de données pour ce jour : le collecteur n'a pas donné signe de vie depuis."
  if (date === today) return "Pas de jeu aujourd'hui."
  if (date === addDays(today, -1)) return 'Pas de jeu hier.'
  return 'Pas de jeu ce jour-là.'
}
