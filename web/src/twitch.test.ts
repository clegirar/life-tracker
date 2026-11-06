import { describe, expect, it } from 'vitest'
import type { DayStream, SourcesHealth } from './api'
import { channelRows, channelUrl, newStreamChannels, overlapText, streamMinutesText, twitchEmptyText, twitchSilent, watchedTwitch } from './twitch'

const DAY: DayStream = {
  date: '2026-10-14',
  minutes: 150,
  summed_minutes: 175,
  segments: 210,
  channels: [
    { key: 'chainefictivea', name: 'ChaineFictiveA', minutes: 120, contents: ['live', 'vod'], first_time: '20:00', last_time: '22:00' },
    { key: 'chainefictiveb', name: 'ChaineFictiveB', minutes: 50, contents: ['clip'], first_time: '21:10', last_time: '22:00' },
  ],
  unknown_minutes: 5,
  discoveries: [{ level: 'channel', key: 'chainefictiveb', name: 'ChaineFictiveB' }],
}

function health(overrides: { silent?: boolean; connected?: boolean; seen?: string | null } = {}): SourcesHealth {
  const { silent = false, connected = true, seen = '2026-10-20T08:00:00Z' } = overrides
  return {
    sources: connected ? [{ name: 'twitch', last_seen_at: seen, silence_threshold_seconds: 259_200, silent }] : [],
    metrics: [],
    errors: 0,
  }
}

describe('le temps', () => {
  it("dit l'union, et jamais zéro pour des segments reçus", () => {
    expect(streamMinutesText(150)).toBe('2 h 30')
    expect(streamMinutesText(45)).toBe('45 min')
    expect(streamMinutesText(0)).toBe("moins d'1 min")
  })
  it('dit le temps à deux streams, à partir d’une minute', () => {
    expect(overlapText(DAY)).toBe('dont 25 min à deux streams')
    expect(overlapText({ ...DAY, summed_minutes: 150 })).toBeNull()
    expect(overlapText({ ...DAY, minutes: 60, summed_minutes: 125 })).toBe('dont 1 h 05 à deux streams')
  })
  it('distingue un jour regardé', () => {
    expect(watchedTwitch(DAY)).toBe(true)
    expect(watchedTwitch({ ...DAY, segments: 0, minutes: 0, summed_minutes: 0, channels: [], unknown_minutes: 0, discoveries: [] })).toBe(false)
  })
})

describe('channelRows', () => {
  const rows = channelRows(DAY)
  it('garde l’ordre du serveur, avec les contenus en français et les heures', () => {
    expect(rows.map((row) => row.name)).toEqual(['ChaineFictiveA', 'ChaineFictiveB', null])
    expect(rows[0]).toMatchObject({ url: 'https://www.twitch.tv/chainefictivea', contents: ['Live', 'Rediffusion'], span: '20:00 – 22:00', minutes: '2 h 00' })
    expect(rows[1].contents).toEqual(['Clip'])
  })
  it('termine par la chaîne inconnue, sans lien ni heure', () => {
    expect(rows[2]).toMatchObject({ name: null, url: null, contents: [], span: null, minutes: '5 min' })
    expect(channelRows({ ...DAY, unknown_minutes: 0 })).toHaveLength(2)
  })
  it('ne fait un lien que d’un login valide', () => {
    expect(channelUrl('abc_123')).toBe('https://www.twitch.tv/abc_123')
    for (const login of ['', 'Majuscule', 'a/b', '../x', 'a b', 'x'.repeat(26), 'a?b=c']) expect(channelUrl(login)).toBeNull()
  })
})

describe('newStreamChannels', () => {
  it('donne les chaînes découvertes ce jour-là', () => {
    expect(newStreamChannels(DAY).map((d) => d.name)).toEqual(['ChaineFictiveB'])
  })
})

describe('twitchEmptyText', () => {
  const today = '2026-10-20'
  it('sans source, sans le mois, ou avant la première donnée', () => {
    expect(twitchEmptyText('2026-10-14', today, null, 0)).toBe('Aucun stream reçu pour ce jour.')
    expect(twitchEmptyText('2026-10-14', today, health(), undefined)).toBe('Aucun stream reçu pour ce jour.')
    expect(twitchEmptyText('2026-10-14', today, health({ connected: false }), null)).toBe("Twitch n'est pas encore relié.")
    expect(twitchEmptyText('2026-09-30', today, health(), null)).toBe('Pas de données Twitch à cette date.')
  })
  it('après le dernier signal de Firefox : on ne sait pas encore', () => {
    expect(twitchEmptyText('2026-10-19', today, health({ seen: '2026-10-18T21:00:00Z' }), 0)).toMatch(/^Pas encore de données pour ce jour/)
    expect(twitchEmptyText('2026-10-19', today, health({ seen: null }), 0)).toMatch(/^Pas encore de données pour ce jour/)
    // 23 h 30 UTC le 18 : déjà le 19 à Paris.
    expect(twitchEmptyText('2026-10-19', today, health({ seen: '2026-10-18T23:30:00Z' }), 0)).toBe('Pas de Twitch hier.')
  })
  it('jusqu’au dernier signal : pas de Twitch', () => {
    expect(twitchEmptyText('2026-10-10', today, health(), 0)).toBe('Pas de Twitch ce jour-là.')
    expect(twitchEmptyText('2026-10-19', today, health(), 0)).toBe('Pas de Twitch hier.')
    expect(twitchEmptyText(today, today, health(), 0)).toBe("Pas de Twitch aujourd'hui.")
  })
})

describe('twitchSilent', () => {
  it('suit la source Twitch seule', () => {
    expect(twitchSilent(null)).toBe(false)
    expect(twitchSilent(health())).toBe(false)
    expect(twitchSilent(health({ silent: true }))).toBe(true)
  })
})
