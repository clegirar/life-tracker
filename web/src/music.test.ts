import { describe, expect, it } from 'vitest'
import type { SourcesHealth } from './api'
import { formatMinutes, musicAlert, musicEmptyText, plural } from './music'

function health(overrides: { silent?: boolean; stale?: boolean; first?: string | null; connected?: boolean } = {}): SourcesHealth {
  const { silent = false, stale = false, first = '2026-10-01T10:00:00Z', connected = true } = overrides
  return {
    sources: connected
      ? [{ name: 'lastfm', last_seen_at: null, silence_threshold_seconds: 21_600, silent }]
      : [],
    metrics: [
      {
        metric: 'music.listen',
        first_event_at: first,
        last_event_at: first,
        freshness_threshold_seconds: 259_200,
        stale,
        coverage: first && connected ? [{ source: 'lastfm', first_at: first, last_at: first }] : [],
      },
    ],
    errors: 0,
  }
}

describe('formatMinutes', () => {
  it('ne confond pas zéro et inconnu', () => {
    expect(formatMinutes(null)).toBe('–')
    expect(formatMinutes(0)).toBe('0 min')
  })
  it('passe aux heures', () => {
    expect(formatMinutes(44)).toBe('44 min')
    expect(formatMinutes(60)).toBe('1 h 00')
    expect(formatMinutes(125)).toBe('2 h 05')
  })
})

it('accorde', () => {
  expect(plural(1, 'écoute')).toBe('1 écoute')
  expect(plural(3, 'écoute')).toBe('3 écoutes')
})

describe('musicAlert', () => {
  it('se tait quand tout va bien ou quand on ne sait pas', () => {
    expect(musicAlert(null)).toBeNull()
    expect(musicAlert(health())).toBeNull()
  })
  it('signale un connecteur muet, avec son seuil', () => {
    expect(musicAlert(health({ silent: true }))?.text).toContain('plus de 6 h')
  })
  it('alerte douce après 3 jours sans écoute', () => {
    expect(musicAlert(health({ stale: true }))?.title).toBe('Musique à vérifier.')
    expect(musicAlert(health({ stale: true }))?.text).toContain('3 jours')
  })
  it('la panne passe avant', () => {
    expect(musicAlert(health({ silent: true, stale: true }))?.title).toBe('Last.fm ne répond plus.')
  })
  it('suit une source remplacée, mais pas un homonyme', () => {
    const renamed = health({ silent: true })
    renamed.sources[0].name = 'lastfm-2'
    expect(musicAlert(renamed)).not.toBeNull()
    renamed.sources[0].name = 'lastfmish'
    expect(musicAlert(renamed)).toBeNull()
  })
})

describe('musicEmptyText', () => {
  const today = '2026-10-31'
  it('une vraie journée sans écoute', () => {
    expect(musicEmptyText(today, today, health())).toBe("Aucune écoute aujourd'hui.")
    expect(musicEmptyText('2026-10-30', today, health())).toBe('Aucune écoute hier.')
    expect(musicEmptyText('2026-10-20', today, health())).toBe('Aucune écoute ce jour-là.')
  })
  it('ne prétend jamais à une journée vide quand les données manquent', () => {
    for (const text of [
      musicEmptyText(today, today, null),
      musicEmptyText(today, today, health({ connected: false })),
      musicEmptyText(today, today, health({ silent: true })),
      musicEmptyText('2026-09-01', today, health()),
      musicEmptyText('2026-10-20', today, health({ first: null })),
    ]) {
      expect(text).not.toMatch(/^Aucune écoute (aujourd|hier|ce jour)/)
    }
  })
  it("compare en jours de Paris : 23 h 30 UTC appartient déjà au lendemain", () => {
    const first = health({ first: '2026-10-09T23:30:00Z' })
    expect(musicEmptyText('2026-10-10', today, first)).toBe('Aucune écoute ce jour-là.')
    expect(musicEmptyText('2026-10-09', today, first)).toBe('Pas de données musicales à cette date.')
  })
})

describe('un historique importé', () => {
  const withHistory = (silent = false): SourcesHealth => {
    const base = health({ silent, first: '2026-10-20T10:00:00Z' })
    base.metrics[0].first_event_at = '2022-01-26T10:00:00Z'
    base.metrics[0].coverage = [
      { source: 'apple', first_at: '2022-01-26T10:00:00Z', last_at: '2026-10-15T10:00:00Z' },
      { source: 'lastfm', first_at: '2026-10-20T10:00:00Z', last_at: '2026-10-30T10:00:00Z' },
    ]
    return base
  }
  const today = '2026-11-01'

  it("dit qu'un jour de sa période n'a pas d'écoute, même si Last.fm se tait aujourd'hui", () => {
    expect(musicEmptyText('2023-05-01', today, withHistory())).toBe('Aucune écoute ce jour-là.')
    expect(musicEmptyText('2023-05-01', today, withHistory(true))).toBe('Aucune écoute ce jour-là.')
  })
  it("ne confond pas le trou entre l'export et Last.fm avec un silence", () => {
    expect(musicEmptyText('2026-10-17', today, withHistory())).toBe('Pas de données musicales à cette date.')
  })
  it('ne sait rien avant le début de son historique', () => {
    expect(musicEmptyText('2021-05-01', today, withHistory())).toBe('Pas de données musicales à cette date.')
  })
  it('laisse Last.fm répondre après son démarrage', () => {
    expect(musicEmptyText('2026-10-25', today, withHistory())).toBe('Aucune écoute ce jour-là.')
  })
})
