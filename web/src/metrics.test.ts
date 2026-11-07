import { describe, expect, it } from 'vitest'
import type { MonthDay, MonthMetric } from './api'
import { dayActivity, effectiveSelection, metricChips, metricTone, setAll, toggleMetric, visibleDots } from './metrics'

const ALL = ['coffee', 'music.listen']

describe('le choix des métriques', () => {
  it('jamais choisi ou disparu du registre : toutes les métriques', () => {
    expect(effectiveSelection(null, ALL)).toEqual(ALL)
    expect(effectiveSelection(['the'], ALL)).toEqual(ALL)
  })

  it('vide : aucune métrique', () => {
    expect(effectiveSelection([], ALL)).toEqual([])
    expect(effectiveSelection([], [])).toEqual([])
  })

  it("garde l'ordre du registre et oublie une métrique disparue", () => {
    expect(effectiveSelection(['music.listen', 'coffee'], ALL)).toEqual(ALL)
    expect(effectiveSelection(['the', 'music.listen'], ALL)).toEqual(['music.listen'])
  })

  it('ajoute et retire, la dernière comprise', () => {
    expect(toggleMetric(ALL, 'coffee', ALL)).toEqual(['music.listen'])
    expect(toggleMetric(['music.listen'], 'music.listen', ALL)).toEqual([])
    expect(toggleMetric([], 'music.listen', ALL)).toEqual(['music.listen'])
    expect(toggleMetric(['music.listen'], 'coffee', ALL)).toEqual(ALL)
  })

  it('tout cocher, tout décocher', () => {
    expect(setAll(ALL, true)).toEqual(ALL)
    expect(setAll(ALL, true)).not.toBe(ALL)
    expect(setAll(ALL, false)).toEqual([])
  })
})

// Le registre tel que le mois le rend, dans l'ordre d'affichage.
const REGISTRY = ['coffee', 'music.listen', 'steps', 'workout', 'sleep', 'calories.burned', 'game.play']
const GARMIN = ['steps', 'workout', 'sleep', 'calories.burned']
const registry = (ids: string[]): MonthMetric[] =>
  ids.map((metric) => ({ metric, label: metric, unit: null }))

describe('la pastille « Activité »', () => {
  it('remplace Pas, Séances, Sommeil et Calories, à la place de la première', () => {
    const chips = metricChips(registry(REGISTRY))
    expect(chips.map((chip) => chip.label)).toEqual(['coffee', 'music.listen', 'Activité', 'game.play'])
    expect(chips[2]).toEqual({ id: 'steps', label: 'Activité', tone: 'heat-steps', metrics: GARMIN })
    expect(chips[0]).toEqual({ id: 'coffee', label: 'coffee', tone: 'heat-coffee', metrics: ['coffee'] })
  })

  it('ne coche que les métriques du registre', () => {
    const chips = metricChips(registry(['coffee', 'sleep', 'steps']))
    expect(chips.map((chip) => [chip.label, chip.metrics])).toEqual([
      ['coffee', ['coffee']],
      ['Activité', ['sleep', 'steps']],
    ])
    expect(toggleMetric(['coffee'], 'sleep', ['coffee', 'sleep', 'steps'])).toEqual(['coffee', 'sleep', 'steps'])
  })

  it('coche et décoche les quatre ensemble, quelle que soit la métrique de l’appui', () => {
    expect(toggleMetric(['coffee'], 'steps', REGISTRY)).toEqual(['coffee', ...GARMIN])
    expect(toggleMetric(['coffee', ...GARMIN], 'steps', REGISTRY)).toEqual(['coffee'])
    expect(toggleMetric(['coffee', ...GARMIN], 'sleep', REGISTRY)).toEqual(['coffee'])
    expect(toggleMetric(GARMIN, 'steps', REGISTRY)).toEqual([])
  })

  it('un ancien choix d’une partie du groupe compte pour tout le groupe', () => {
    expect(effectiveSelection(['coffee', 'sleep'], REGISTRY)).toEqual(['coffee', ...GARMIN])
    expect(effectiveSelection(['coffee'], REGISTRY)).toEqual(['coffee'])
    // Allumée par une seule métrique, la pastille s'éteint d'un appui, les quatre avec elle.
    expect(toggleMetric(['coffee', 'sleep'], 'steps', REGISTRY)).toEqual(['coffee'])
    expect(setAll(REGISTRY, false)).toEqual([])
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
