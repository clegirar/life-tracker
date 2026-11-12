import { describe, expect, it } from 'vitest'
import type { DayGames, GameMatch, SourcesHealth } from './api'
import {
  gameRows,
  gameSections,
  gamesEmptyText,
  matchRecord,
  matchRow,
  parseFolded,
  placementText,
  playedGames,
  playMinutesText,
  sectionSummary,
  silenceText,
  silentGameServices,
  storeUrl,
  toggleFolded,
  totalText,
} from './games'

const DAY: DayGames = {
  date: '2026-10-14',
  minutes: 100,
  segments: 3,
  achievements: 2,
  discoveries: [{ level: 'game', key: '777', name: 'Autre Jeu Fictif' }],
  games: [
    {
      key: '4242',
      name: 'Jeu Fictif',
      minutes: 70,
      first_time: '21:15',
      last_time: '22:25',
      total_minutes: 9_610,
      achievements: [{ key: '4242:ACH_FICTIF', name: 'Succès fictif', time: '21:50' }],
    },
    { key: '777', name: 'Autre Jeu Fictif', minutes: 30, first_time: '23:45', last_time: '00:15', total_minutes: 160, achievements: [] },
    {
      key: '99',
      name: 'Jeu Sans Partie',
      minutes: 0,
      first_time: null,
      last_time: null,
      total_minutes: null,
      achievements: [{ key: '99:ACH', name: 'Succès hors ligne', time: '08:02' }],
    },
  ],
  matches: [],
}

function health(overrides: { silent?: boolean; connected?: boolean; seen?: string | null } = {}): SourcesHealth {
  const { silent = false, connected = true, seen = '2026-10-20T08:00:00Z' } = overrides
  return {
    sources: connected ? [{ name: 'steam', last_seen_at: seen, silence_threshold_seconds: 21_600, silent }] : [],
    metrics: [],
    errors: 0,
  }
}

describe('le temps', () => {
  it('dit le temps du jour, et jamais zéro pour une partie reçue', () => {
    expect(playMinutesText(100)).toBe('1 h 40')
    expect(playMinutesText(40)).toBe('40 min')
    expect(playMinutesText(0)).toBe("moins d'1 min")
  })
  it('dit le total de Steam, en heures entières dès 10 h', () => {
    expect(totalText(45)).toBe('45 min au total')
    expect(totalText(160)).toBe('2 h 40 au total')
    expect(totalText(599)).toBe('9 h 59 au total')
    expect(totalText(9_610)).toBe('160 h au total')
    expect(totalText(120_000)).toBe('2 000 h au total')
    expect(totalText(null)).toBeNull()
    expect(totalText(0)).toBeNull()
  })
  it('distingue un jour joué, ou à succès seul', () => {
    expect(playedGames(DAY)).toBe(true)
    const empty = { ...DAY, minutes: 0, segments: 0, achievements: 0, games: [] }
    expect(playedGames(empty)).toBe(false)
    expect(playedGames({ ...empty, achievements: 1, games: [DAY.games[2]] })).toBe(true)
  })
})

describe('les découvertes', () => {
  it('marquent la ligne du jeu joué pour la première fois, et elle seule', () => {
    expect(gameRows(DAY).map((row) => [row.key, row.discovery])).toEqual([
      ['4242', false],
      ['777', true],
      ['99', false],
    ])
    expect(gameRows({ ...DAY, discoveries: [] }).some((row) => row.discovery)).toBe(false)
  })
})

describe('gameRows', () => {
  const rows = gameRows(DAY)
  it('garde l’ordre du serveur, avec les heures, le total et les succès', () => {
    expect(rows.map((row) => row.name)).toEqual(['Jeu Fictif', 'Autre Jeu Fictif', 'Jeu Sans Partie'])
    expect(rows[0]).toMatchObject({
      url: 'https://store.steampowered.com/app/4242/',
      detail: '21:15 – 22:25 · 160 h au total',
      minutes: '1 h 10',
      achievements: [{ key: '4242:ACH_FICTIF', name: 'Succès fictif', time: '21:50' }],
    })
    // À cheval sur minuit : ses vraies heures.
    expect(rows[1]).toMatchObject({ detail: '23:45 – 00:15 · 2 h 40 au total', minutes: '30 min', achievements: [] })
  })
  it('liste un jeu qui n’a qu’un succès, sans temps ni heure', () => {
    expect(rows[2]).toMatchObject({ detail: '', minutes: null })
    expect(rows[2].achievements).toHaveLength(1)
  })
  it('ne fait un lien que d’un appid valide', () => {
    expect(storeUrl('4242')).toBe('https://store.steampowered.com/app/4242/')
    for (const appid of ['', '0', '012', 'abc', '1/2', '../x', '1'.repeat(11)]) expect(storeUrl(appid)).toBeNull()
  })
})

describe('silentGameServices', () => {
  it('ne regarde que les sources des jeux', () => {
    expect(silentGameServices(health({ silent: true }))).toEqual(['Steam'])
    expect(silentGameServices(health())).toEqual([])
    expect(silentGameServices(null)).toEqual([])
    const source = (name: string) => ({ name, last_seen_at: null, silence_threshold_seconds: 1, silent: true })
    expect(silentGameServices({ sources: [source('garmin')], metrics: [], errors: 0 })).toEqual([])
    expect(silentGameServices({ sources: [source('valorant'), source('riot-2'), source('steam')], metrics: [], errors: 0 })).toEqual(['Steam', 'Riot Games', 'VALORANT'])
  })
  it('le dit en une phrase', () => {
    expect(silenceText(['Steam'])).toBe('Le collecteur Steam ne donne plus signe de vie.')
    expect(silenceText(['Steam', 'Riot Games'])).toBe('Les collecteurs Steam et Riot Games ne donnent plus signe de vie.')
    expect(silenceText(['Steam', 'Riot Games', 'VALORANT'])).toBe('Les collecteurs Steam, Riot Games et VALORANT ne donnent plus signe de vie.')
  })
})

const NBSP = '\u00a0'

const match = (overrides: Partial<GameMatch>): GameMatch => ({
  key: 'lol:EUW1_1',
  time: '20:05',
  game: 'riot:lol',
  game_name: 'League of Legends',
  queue: 'Classée solo/duo',
  result: 'win',
  character: 'Champion Fictif',
  position: 'MIDDLE',
  map: null,
  placement: null,
  rounds_won: null,
  rounds_lost: null,
  minutes: 31,
  ...overrides,
})
const tft = (key: string, placement: number) =>
  match({ key, game: 'riot:tft', game_name: 'Teamfight Tactics', queue: 'Double Up', result: null, character: null, position: null, placement, minutes: 28 })

describe('les parties', () => {
  it('disent le résultat, ce que j’ai joué, la file et la durée', () => {
    expect(matchRow(match({}))).toEqual({
      key: 'lol:EUW1_1',
      time: '20:05',
      result: 'Victoire',
      tone: 'win',
      title: 'Champion Fictif · Milieu',
      detail: 'Classée solo/duo',
      minutes: '31 min',
    })
    expect(matchRow(match({ result: 'loss', position: 'UTILITY' }))).toMatchObject({ result: 'Défaite', tone: 'loss', title: 'Champion Fictif · Soutien' })
    expect(matchRow(match({ result: 'remake', minutes: 3, position: null }))).toMatchObject({ result: 'Remake', tone: 'neutral', title: 'Champion Fictif' })
  })
  it('en TFT : la place, la file pour titre', () => {
    expect(placementText(1)).toBe('1re')
    expect(placementText(4)).toBe('4e')
    expect(matchRow(tft('tft:1', 1))).toMatchObject({ result: '1re', tone: 'win', title: 'Double Up', detail: null })
    expect(matchRow(tft('tft:2', 6))).toMatchObject({ result: '6e', tone: 'loss' })
  })
  it('en VALORANT : l’agent, la carte, les rounds ; l’égalité', () => {
    const valorant = match({ game: 'riot:valorant', game_name: 'VALORANT', queue: 'Competitive', character: 'Agent Fictif', position: null, map: 'Carte Fictive', rounds_won: 13, rounds_lost: 8 })
    expect(matchRow(valorant)).toMatchObject({ result: 'Victoire', title: 'Agent Fictif · Carte Fictive', detail: 'Competitive · 13–8' })
    expect(matchRow({ ...valorant, result: 'draw', rounds_won: 12, rounds_lost: 12 })).toMatchObject({ result: 'Égalité', tone: 'neutral', detail: 'Competitive · 12–12' })
    // Un deathmatch : ni résultat ni rounds.
    expect(matchRow({ ...valorant, result: null, rounds_won: null, rounds_lost: null, queue: 'Deathmatch' })).toMatchObject({ result: null, tone: 'neutral', detail: 'Deathmatch' })
  })
  it('font le bilan du jour, par jeu', () => {
    expect(matchRecord([])).toBeNull()
    const lol = [match({}), match({ result: 'win' }), match({ result: 'loss' }), match({ result: 'remake' })]
    expect(matchRecord(lol)).toBe(`2${NBSP}V${NBSP}– 1${NBSP}D · 1${NBSP}remake`)
    expect(matchRecord([match({ result: 'remake' }), match({ result: 'remake' })])).toBe(`2${NBSP}remakes`)
    expect(matchRecord([match({ result: 'draw' }), match({ result: 'loss' })])).toBe(`0${NBSP}V${NBSP}– 1${NBSP}D${NBSP}– 1${NBSP}N`)
    expect(matchRecord([tft('a', 1), tft('b', 5), tft('c', 4)])).toBe(`Top${NBSP}4${NBSP}: 2${NBSP}sur${NBSP}3`)
  })
  it('se rangent sous leur jeu, avec le nombre et le bilan', () => {
    const day: DayGames = {
      ...DAY,
      games: [{ key: 'riot:lol', name: 'League of Legends', minutes: 62, first_time: '20:05', last_time: '21:10', total_minutes: null, achievements: [] }, DAY.games[0]],
      matches: [match({ key: 'a' }), match({ key: 'b', time: '20:40', result: 'loss' }), tft('c', 3)],
    }
    const rows = gameRows(day)
    expect(rows.map((row) => row.key)).toEqual(['riot:lol', '4242', 'riot:tft'])
    expect(rows[0]).toMatchObject({ url: null, detail: `20:05 – 21:10 · 2${NBSP}parties · 1${NBSP}V${NBSP}– 1${NBSP}D`, minutes: '1 h 02' })
    expect(rows[0].matches.map((m) => m.key)).toEqual(['a', 'b'])
    expect(rows[1].matches).toEqual([])
    // Une partie dont le jeu n'a pas de tranche ce jour-là : sa ligne, sans temps.
    expect(rows[2]).toMatchObject({ name: 'Teamfight Tactics', minutes: null, detail: `1${NBSP}partie · Top${NBSP}4${NBSP}: 1${NBSP}sur${NBSP}1` })
    // Deux sections : Steam, puis Riot (clés `riot:`), chacune dans l'ordre du serveur.
    const sections = gameSections(day)
    expect(sections.steam.map((row) => row.key)).toEqual(['4242'])
    expect(sections.riot.map((row) => row.key)).toEqual(['riot:lol', 'riot:tft'])
    expect(gameSections(DAY).riot).toEqual([])
    expect(playedGames({ ...DAY, segments: 0, achievements: 0, games: [], matches: [tft('c', 3)] })).toBe(true)
  })
})

describe('gamesEmptyText', () => {
  const today = '2026-10-20'
  it('sans source, sans le mois, ou avant le premier relevé', () => {
    expect(gamesEmptyText('2026-10-14', today, null, 0)).toBe('Aucune partie reçue pour ce jour.')
    expect(gamesEmptyText('2026-10-14', today, health(), undefined)).toBe('Aucune partie reçue pour ce jour.')
    expect(gamesEmptyText('2026-10-14', today, health({ connected: false }), null)).toBe("Aucun jeu n'est encore relié.")
    expect(gamesEmptyText('2026-09-30', today, health(), null)).toBe('Pas de données de jeu à cette date.')
  })
  it('après le dernier passage du collecteur : on ne sait pas encore', () => {
    expect(gamesEmptyText('2026-10-19', today, health({ seen: '2026-10-18T21:00:00Z' }), 0)).toMatch(/^Pas encore de données pour ce jour/)
    expect(gamesEmptyText('2026-10-19', today, health({ seen: null }), 0)).toMatch(/^Pas encore de données pour ce jour/)
    // 23 h 30 UTC le 18 : déjà le 19 à Paris.
    expect(gamesEmptyText('2026-10-19', today, health({ seen: '2026-10-18T23:30:00Z' }), 0)).toBe('Pas de jeu hier.')
  })
  it('jusqu’au dernier passage : pas de jeu', () => {
    expect(gamesEmptyText('2026-10-10', today, health(), 0)).toBe('Pas de jeu ce jour-là.')
    expect(gamesEmptyText('2026-10-19', today, health(), 0)).toBe('Pas de jeu hier.')
    expect(gamesEmptyText(today, today, health(), 0)).toBe("Pas de jeu aujourd'hui.")
  })
})

describe('les sections pliées', () => {
  it('résument ce qu’elles cachent : le temps et les jeux pour Steam, le temps et les parties pour Riot', () => {
    expect(sectionSummary(gameRows(DAY), 'steam')).toBe(`1 h 40 · 3${NBSP}jeux`)
    expect(sectionSummary(gameRows(DAY).slice(0, 1), 'steam')).toBe(`1 h 10 · 1${NBSP}jeu`)
    // Un jeu qui n'a qu'un succès : pas de temps.
    expect(sectionSummary(gameRows(DAY).slice(2), 'steam')).toBe(`1${NBSP}jeu`)
    const day: DayGames = {
      ...DAY,
      games: [{ key: 'riot:lol', name: 'League of Legends', minutes: 62, first_time: '20:05', last_time: '21:10', total_minutes: null, achievements: [] }],
      matches: [match({ key: 'a' }), match({ key: 'b' }), tft('c', 3)],
    }
    expect(sectionSummary(gameSections(day).riot, 'riot')).toBe(`1 h 02 · 3${NBSP}parties`)
  })
  it('se gardent en liste, lue sans confiance', () => {
    expect(parseFolded(null)).toEqual([])
    expect(parseFolded('["riot"]')).toEqual(['riot'])
    expect(parseFolded('["riot","steam","autre"]')).toEqual(['steam', 'riot'])
    expect(parseFolded('{')).toEqual([])
    expect(parseFolded('"riot"')).toEqual([])
    expect(toggleFolded([], 'riot')).toEqual(['riot'])
    expect(toggleFolded(['riot', 'steam'], 'riot')).toEqual(['steam'])
  })
})
