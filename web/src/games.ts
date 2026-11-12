// Ce que la carte des jeux dit d'un jour : Steam, League of Legends, Teamfight Tactics et VALORANT.
// Steam ne donne que des compteurs cumulés : le temps d'un jour vient de l'écart entre deux relevés du
// collecteur, placé à la fin de la partie. Les succès arrivent avec leur vraie date. Riot (LoL, TFT) et
// HenrikDev (VALORANT) donnent chaque partie : elle se range sous son jeu, avec le bilan du jour. La
// carte a deux sections, « Steam » et « Riot ». Une journée sans données ne se confond jamais avec une
// journée sans jeu.

import type { DayGame, DayGames, GameMatch, SourcesHealth } from './api'
import { formatMinutes, plural } from './music'
import { addDays, parisDate } from './time'

// Les sources des jeux ; un remplacement de token crée une nouvelle source : `steam-2`.
const GAME_SOURCES: [RegExp, string][] = [
  [/^steam\b/, 'Steam'],
  [/^riot\b/, 'Riot Games'],
  [/^valorant\b/, 'VALORANT'],
]
const serviceOf = (name: string) => GAME_SOURCES.find(([pattern]) => pattern.test(name))?.[1] ?? null

const NUMBER = new Intl.NumberFormat('fr-FR', { maximumFractionDigits: 0 })
/** L'espace insécable : « 4 V – 2 D · 1 remake » ne se coupe jamais entre un nombre et son mot. */
const NBSP = '\u00a0'
const count = (n: number, word: string) => plural(n, word).replace(' ', NBSP)

/** Rien n'a été reçu ce jour-là, ni temps, ni succès, ni partie : la carte dit pourquoi, sans chiffre. */
export const playedGames = (day: DayGames) => day.segments > 0 || day.achievements > 0 || day.matches.length > 0

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

/** Le ton d'un résultat : la victoire en vert, la défaite en rouge, le reste sans couleur. */
export type MatchTone = 'win' | 'loss' | 'neutral'

/** Une partie, sous son jeu. */
export interface MatchRow {
  key: string
  time: string
  /** « Victoire », « Défaite », « Remake », « Égalité », ou la place en TFT : « 1re », « 4e ». */
  result: string | null
  tone: MatchTone
  /** Ce que j'ai joué : « Ahri · Milieu », « Jett · Ascent » ; en TFT, la file. */
  title: string
  /** La file (LoL, VALORANT) et les rounds : « Compétitif · 13–8 » ; `null` en TFT. */
  detail: string | null
  minutes: string
}

/** Une ligne de la liste des jeux. */
export interface GameRow {
  key: string
  name: string
  url: string | null
  /** « 21:15 – 21:55 · 2 h 40 au total », « 20:00 – 21:24 · 5 parties · 3 V – 2 D » ; vide si rien n'est connu. */
  detail: string
  /** `null` : un jeu qui n'a qu'un succès ce jour-là, sans partie reçue. */
  minutes: string | null
  achievements: { key: string; name: string; time: string }[]
  /** Les parties commencées ce jour-là, par heure (LoL, TFT, VALORANT). */
  matches: MatchRow[]
  /** Joué ce jour-là pour la première fois (Steam). */
  discovery: boolean
  /** Le temps du jour, en minutes ; 0 sans tranche ce jour-là. */
  playedMinutes: number
}

/** Les deux sections de la carte : Steam, puis Riot (LoL, TFT, VALORANT, sous les clés `riot:`). */
export interface GameSections {
  steam: GameRow[]
  riot: GameRow[]
}

const POSITIONS: Record<string, string> = {
  TOP: 'Haut',
  JUNGLE: 'Jungle',
  MIDDLE: 'Milieu',
  BOTTOM: 'Bas',
  UTILITY: 'Soutien',
}

/** « 1re », « 2e » : la place en TFT. */
export const placementText = (placement: number) => `${placement}${placement === 1 ? 're' : 'e'}`

const RESULTS: Record<NonNullable<GameMatch['result']>, [string, MatchTone]> = {
  win: ['Victoire', 'win'],
  loss: ['Défaite', 'loss'],
  remake: ['Remake', 'neutral'],
  draw: ['Égalité', 'neutral'],
}

/** Une place de TFT dans les quatre premières compte comme une partie gagnée, comme le jeu la colore. */
const TOP_FOUR = 4

export function matchRow(match: GameMatch): MatchRow {
  const [result, tone]: [string | null, MatchTone] =
    match.result !== null
      ? RESULTS[match.result]
      : match.placement !== null
        ? [placementText(match.placement), match.placement <= TOP_FOUR ? 'win' : 'loss']
        : [null, 'neutral']
  const position = match.position ? (POSITIONS[match.position] ?? null) : null
  const who = [match.character, position ?? match.map].filter(Boolean).join(' · ')
  const rounds = match.rounds_won !== null && match.rounds_lost !== null ? `${match.rounds_won}–${match.rounds_lost}` : null
  // TFT : ni personnage ni carte, la file en tient lieu.
  const title = who || match.queue || match.game_name
  const detail = [who ? match.queue : null, rounds].filter(Boolean).join(' · ') || null
  return { key: match.key, time: match.time, result, tone, title, detail, minutes: playMinutesText(match.minutes) }
}

/**
 * Le bilan d'un jeu sur le jour : « 3 V – 2 D », « 3 V – 2 D – 1 N · 1 remake » ; en TFT, les places
 * dans les quatre premières : « Top 4 : 2 sur 3 ». `null` sans partie. Des espaces insécables entre
 * chaque nombre et son mot, et autour des tirets : le bilan ne se coupe qu'après un « · ».
 */
export function matchRecord(matches: GameMatch[]): string | null {
  if (matches.length === 0) return null
  const placed = matches.filter((m) => m.result === null && m.placement !== null)
  if (placed.length === matches.length) {
    return `Top${NBSP}4${NBSP}: ${placed.filter((m) => m.placement! <= TOP_FOUR).length}${NBSP}sur${NBSP}${placed.length}`
  }
  const tally = (result: GameMatch['result']) => matches.filter((m) => m.result === result).length
  const [wins, losses, draws, remakes] = [tally('win'), tally('loss'), tally('draw'), tally('remake')]
  const record =
    wins + losses + draws > 0 ? [`${wins}${NBSP}V`, `${losses}${NBSP}D`, ...(draws > 0 ? [`${draws}${NBSP}N`] : [])].join(`${NBSP}– `) : null
  return [record, remakes > 0 ? count(remakes, 'remake') : null].filter(Boolean).join(' · ') || null
}

function gameRow(game: DayGame, matches: GameMatch[], discovered: boolean): GameRow {
  const played = game.first_time !== null && game.last_time !== null
  const span = played ? `${game.first_time} – ${game.last_time}` : null
  const parties = matches.length > 0 ? count(matches.length, 'partie') : null
  return {
    key: game.key,
    name: game.name,
    url: storeUrl(game.key),
    detail: [span, totalText(game.total_minutes), parties, matchRecord(matches)].filter(Boolean).join(' · '),
    minutes: played ? playMinutesText(game.minutes) : null,
    achievements: game.achievements,
    matches: matches.map(matchRow),
    discovery: discovered,
    playedMinutes: game.minutes,
  }
}

/** Un jeu de Riot ou de VALORANT : sa clé porte le préfixe `riot:`, qu'aucun appid de Steam ne prend. */
export const isRiotGame = (key: string) => key.startsWith('riot:')

/**
 * Les jeux dans l'ordre du serveur : joués par temps, puis ceux qui n'ont qu'un succès. Chaque partie
 * se range sous son jeu ; une partie dont le jeu manque à la liste (aucune tranche reçue ce jour-là)
 * lui ajoute une ligne, sans temps.
 */
export function gameRows(day: DayGames): GameRow[] {
  const matchesOf = (key: string) => day.matches.filter((m) => m.game === key)
  const discovered = (key: string) => day.discoveries.some((found) => found.key === key)
  const rows = day.games.map((game) => gameRow(game, matchesOf(game.key), discovered(game.key)))
  for (const match of day.matches) {
    if (rows.some((row) => row.key === match.game)) continue
    const game: DayGame = { key: match.game, name: match.game_name, minutes: 0, first_time: null, last_time: null, total_minutes: null, achievements: [] }
    rows.push(gameRow(game, matchesOf(match.game), false))
  }
  return rows
}

/** Les lignes réparties entre les deux sections, chacune dans l'ordre du serveur. Une section vide ne s'affiche pas. */
export function gameSections(day: DayGames): GameSections {
  const rows = gameRows(day)
  return { steam: rows.filter((row) => !isRiotGame(row.key)), riot: rows.filter((row) => isRiotGame(row.key)) }
}

/** Une section de la carte, pliable. */
export type GameSectionId = keyof GameSections

/**
 * Ce que dit l'intertitre d'une section pliée, pour savoir ce qu'on cache : « 1 h 05 · 2 jeux » pour
 * Steam, « 5 h 49 · 12 parties » pour Riot. Le temps est la somme des jeux (deux jeux à la fois
 * comptent chacun). `null` sans rien à dire.
 */
export function sectionSummary(rows: GameRow[], id: GameSectionId): string | null {
  const minutes = rows.reduce((sum, row) => sum + row.playedMinutes, 0)
  const time = rows.some((row) => row.minutes !== null) ? playMinutesText(minutes) : null
  const matches = rows.reduce((sum, row) => sum + row.matches.length, 0)
  const tail = id === 'riot' ? (matches > 0 ? count(matches, 'partie') : null) : `${rows.length}${NBSP}${rows.length > 1 ? 'jeux' : 'jeu'}`
  return [time, tail].filter(Boolean).join(' · ') || null
}

/** Les sections pliées, gardées d'un jour et d'une visite à l'autre : une liste lue sans confiance. */
export function parseFolded(raw: string | null): GameSectionId[] {
  try {
    const value: unknown = JSON.parse(raw ?? '[]')
    return Array.isArray(value) ? (['steam', 'riot'] as const).filter((id) => value.includes(id)) : []
  } catch {
    return []
  }
}

/** Plie ou déplie une section. */
export const toggleFolded = (folded: GameSectionId[], id: GameSectionId): GameSectionId[] =>
  folded.includes(id) ? folded.filter((f) => f !== id) : [...folded, id]

/** Les parties d'un jeu qu'on montre d'emblée ; au-delà, « N autres parties ». */
export const MATCH_LIMIT = 5

/** Les services de jeu dont la source se tait (« Steam », « Riot Games ») : des parties de ce jour peuvent manquer. */
export function silentGameServices(health: SourcesHealth | null): string[] {
  const names = (health?.sources ?? []).filter((source) => source.silent).map((source) => serviceOf(source.name))
  return GAME_SOURCES.map(([, name]) => name).filter((name) => names.includes(name))
}

/** « Le collecteur Steam ne donne plus signe de vie. », « Les collecteurs Steam et Riot Games ne donnent… ». */
export function silenceText(services: string[]): string {
  if (services.length === 1) return `Le collecteur ${services[0]} ne donne plus signe de vie.`
  const list = `${services.slice(0, -1).join(', ')} et ${services.at(-1)}`
  return `Les collecteurs ${list} ne donnent plus signe de vie.`
}

/**
 * Le texte d'une journée sans partie ni succès reçus. `value` est la case du mois pour `game.play` :
 * `null` avant la première donnée, `undefined` tant que le mois n'est pas lu. Les collecteurs passent
 * tous les quarts d'heure : après le dernier passage de chacun, on ne sait pas encore.
 */
export function gamesEmptyText(date: string, today: string, health: SourcesHealth | null, value: number | null | undefined): string {
  if (!health || value === undefined) return 'Aucune partie reçue pour ce jour.'
  const sources = health.sources.filter((source) => serviceOf(source.name) !== null)
  if (sources.length === 0) return "Aucun jeu n'est encore relié."
  if (value === null) return 'Pas de données de jeu à cette date.'
  const seen = sources
    .map((source) => source.last_seen_at)
    .filter((at): at is string => at !== null)
    .map((at) => parisDate(Date.parse(at)))
    .sort()
    .at(-1)
  if (seen === undefined || date > seen) return "Pas encore de données pour ce jour : aucun collecteur n'a donné signe de vie depuis."
  if (date === today) return "Pas de jeu aujourd'hui."
  if (date === addDays(today, -1)) return 'Pas de jeu hier.'
  return 'Pas de jeu ce jour-là.'
}
