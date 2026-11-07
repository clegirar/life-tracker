import { describe, expect, it } from 'vitest'
import type { DayGames, SourcesHealth } from './api'
import { discoveryRows, gameRows, playedSteam, playMinutesText, steamEmptyText, steamSilent, storeUrl, totalText } from './steam'

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
    expect(playedSteam(DAY)).toBe(true)
    const empty = { ...DAY, minutes: 0, segments: 0, achievements: 0, games: [] }
    expect(playedSteam(empty)).toBe(false)
    expect(playedSteam({ ...empty, achievements: 1, games: [DAY.games[2]] })).toBe(true)
  })
})

describe('discoveryRows', () => {
  it('liste les jeux joués pour la première fois, avec leur page du magasin', () => {
    expect(discoveryRows(DAY)).toEqual([{ key: '777', name: 'Autre Jeu Fictif', url: 'https://store.steampowered.com/app/777/' }])
  })
  it('rien un jour sans nouveau jeu : la carte ne montre pas la section', () => {
    expect(discoveryRows({ ...DAY, discoveries: [] })).toEqual([])
  })
  it('pas de lien pour un appid douteux', () => {
    expect(discoveryRows({ ...DAY, discoveries: [{ level: 'game', key: 'x', name: 'Jeu' }] })[0].url).toBeNull()
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

describe('steamSilent', () => {
  it('ne regarde que les sources Steam', () => {
    expect(steamSilent(health({ silent: true }))).toBe(true)
    expect(steamSilent(health())).toBe(false)
    expect(steamSilent(null)).toBe(false)
    expect(steamSilent({ sources: [{ name: 'garmin', last_seen_at: null, silence_threshold_seconds: 1, silent: true }], metrics: [], errors: 0 })).toBe(false)
  })
})

describe('steamEmptyText', () => {
  const today = '2026-10-20'
  it('sans source, sans le mois, ou avant le premier relevé', () => {
    expect(steamEmptyText('2026-10-14', today, null, 0)).toBe('Aucune partie reçue pour ce jour.')
    expect(steamEmptyText('2026-10-14', today, health(), undefined)).toBe('Aucune partie reçue pour ce jour.')
    expect(steamEmptyText('2026-10-14', today, health({ connected: false }), null)).toBe("Steam n'est pas encore relié.")
    expect(steamEmptyText('2026-09-30', today, health(), null)).toBe('Pas de données Steam à cette date.')
  })
  it('après le dernier passage du collecteur : on ne sait pas encore', () => {
    expect(steamEmptyText('2026-10-19', today, health({ seen: '2026-10-18T21:00:00Z' }), 0)).toMatch(/^Pas encore de données pour ce jour/)
    expect(steamEmptyText('2026-10-19', today, health({ seen: null }), 0)).toMatch(/^Pas encore de données pour ce jour/)
    // 23 h 30 UTC le 18 : déjà le 19 à Paris.
    expect(steamEmptyText('2026-10-19', today, health({ seen: '2026-10-18T23:30:00Z' }), 0)).toBe('Pas de jeu hier.')
  })
  it('jusqu’au dernier passage : pas de jeu', () => {
    expect(steamEmptyText('2026-10-10', today, health(), 0)).toBe('Pas de jeu ce jour-là.')
    expect(steamEmptyText('2026-10-19', today, health(), 0)).toBe('Pas de jeu hier.')
    expect(steamEmptyText(today, today, health(), 0)).toBe("Pas de jeu aujourd'hui.")
  })
})
