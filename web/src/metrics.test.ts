import { describe, expect, it } from 'vitest'
import type { MonthDay } from './api'
import { dayActivity, effectiveSelection, metricTone, toggleMetric, visibleDots } from './metrics'

const ALL = ['coffee', 'music.listen']

describe('le choix des métriques', () => {
  it('jamais choisi, vide ou inconnu : toutes les métriques', () => {
    expect(effectiveSelection(null, ALL)).toEqual(ALL)
    expect(effectiveSelection([], ALL)).toEqual(ALL)
    expect(effectiveSelection(['the'], ALL)).toEqual(ALL)
  })

  it("garde l'ordre du registre et oublie une métrique disparue", () => {
    expect(effectiveSelection(['music.listen', 'coffee'], ALL)).toEqual(ALL)
    expect(effectiveSelection(['the', 'music.listen'], ALL)).toEqual(['music.listen'])
  })

  it('ajoute et retire, mais garde toujours la dernière', () => {
    expect(toggleMetric(ALL, 'coffee', ALL)).toEqual(['music.listen'])
    expect(toggleMetric(['music.listen'], 'music.listen', ALL)).toEqual(['music.listen'])
    expect(toggleMetric(['music.listen'], 'coffee', ALL)).toEqual(ALL)
  })
})

describe('les couleurs', () => {
  it('une métrique nouvelle prend la teinte neutre', () => {
    expect(metricTone('music.listen')).toBe('heat-music')
    expect(metricTone('the')).toBe('heat-neutral')
  })
})

const day = (values: Record<string, number | null>): MonthDay => ({ date: '2026-11-02', values, errors: 0, pending: [], to_check: [] })

describe('les points d’une journée', () => {
  it('un point par métrique active, dans l’ordre du registre', () => {
    expect(dayActivity(day({ coffee: 2, 'music.listen': 140 }), ALL)).toEqual({ noData: false, active: ALL })
    expect(dayActivity(day({ coffee: null, 'music.listen': 3 }), ALL)).toEqual({ noData: false, active: ['music.listen'] })
  })

  it('pas de données n’est pas zéro', () => {
    // Aucune donnée pour les métriques choisies : numéro grisé, aucun point.
    expect(dayActivity(day({ coffee: null, 'music.listen': null }), ALL)).toEqual({ noData: true, active: [] })
    expect(dayActivity(undefined, ALL)).toEqual({ noData: true, active: [] })
    // Des données mais rien ce jour-là : numéro normal, aucun point.
    expect(dayActivity(day({ coffee: 0, 'music.listen': null }), ALL)).toEqual({ noData: false, active: [] })
    // Seules comptent les métriques choisies.
    expect(dayActivity(day({ coffee: null, 'music.listen': 5 }), ['coffee'])).toEqual({ noData: true, active: [] })
  })

  it('trois points au plus dans l’année, huit puis « +N » dans le mois', () => {
    const twenty = Array.from({ length: 20 }, (_, i) => `m${i}`)
    expect(visibleDots(twenty, 3)).toEqual({ dots: ['m0', 'm1', 'm2'], more: 17 })
    expect(visibleDots(twenty, 8)).toEqual({ dots: twenty.slice(0, 8), more: 12 })
    expect(visibleDots(twenty.slice(0, 8), 8)).toEqual({ dots: twenty.slice(0, 8), more: 0 })
    expect(visibleDots([], 3)).toEqual({ dots: [], more: 0 })
  })
})
