import { describe, expect, it } from 'vitest'
import type { MonthDay, MonthMetric } from './api'
import { addMonths, cellLabel, cellText, compact, dayNotes, errorCountText, hoursText, marker, monthGrid, monthName, sourceName, valueText } from './calendar'

const METRICS: MonthMetric[] = [
  { metric: 'coffee', label: 'Cafés', unit: 'tasse' },
  { metric: 'music.listen', label: 'Écoutes', unit: 'écoute' },
]

const day = (overrides: Partial<MonthDay> = {}): MonthDay => ({
  date: '2026-10-25',
  values: { coffee: null, 'music.listen': 0 },
  errors: 0,
  pending: [],
  to_check: [],
  ...overrides,
})

describe('monthGrid', () => {
  it('commence le lundi et ne remplit que les jours du mois', () => {
    // Le 1er novembre 2026 est un dimanche : six cases vides avant lui.
    const weeks = monthGrid('2026-11')
    expect(weeks[0]).toEqual([null, null, null, null, null, null, '2026-11-01'])
    expect(weeks.flat().filter(Boolean)).toHaveLength(30)
    expect(weeks.at(-1)!.at(-1)).toBe(null)
    expect(weeks.every((week) => week.length === 7)).toBe(true)
  })

  it('compte les années bissextiles', () => {
    expect(monthGrid('2028-02').flat().filter(Boolean)).toHaveLength(29)
    expect(monthGrid('2026-02').flat().filter(Boolean)).toHaveLength(28)
  })
})

describe('addMonths', () => {
  it("passe d'une année à l'autre", () => {
    expect(addMonths('2026-01', -1)).toBe('2025-12')
    expect(addMonths('2026-12', 1)).toBe('2027-01')
    expect(monthName('2026-11')).toBe('Novembre')
  })
})

describe('marker', () => {
  it("met l'erreur avant l'attente, et l'attente avant la vérification", () => {
    expect(marker(day())).toBe(null)
    expect(marker(day({ to_check: ['music.listen'] }))).toBe('check')
    expect(marker(day({ to_check: ['music.listen'], pending: ['lastfm'] }))).toBe('pending')
    expect(marker(day({ to_check: ['music.listen'], pending: ['lastfm'], errors: 1 }))).toBe('incomplete')
  })
})

describe('les textes', () => {
  it('ne confondent jamais zéro et pas de données', () => {
    expect(valueText(METRICS[0], null)).toBe('cafés : pas de données')
    expect(valueText(METRICS[0], 0)).toBe('0 café')
    expect(valueText(METRICS[0], 3)).toBe('3 cafés')
    expect(valueText(METRICS[1], 1)).toBe('1 écoute')
  })

  it('disent ce qui manque à la journée', () => {
    const notes = dayNotes(day({ errors: 2, pending: ['lastfm-2'], to_check: ['music.listen'] }), METRICS)
    expect(notes).toEqual(['journée incomplète, 2 enregistrements en erreur', 'en attente de Last.fm', 'écoutes à vérifier'])
    expect(sourceName('apple')).toBe('apple')
    expect(cellLabel(day({ errors: 1 }), METRICS, '2026-11-02')).toBe(
      'Dimanche 25 octobre : cafés : pas de données, 0 écoute, journée incomplète, 1 enregistrement en erreur',
    )
  })

  it('disent « pas de données » une fois quand aucune métrique choisie n’en a', () => {
    const empty = day({ values: { coffee: null, 'music.listen': null } })
    expect(cellLabel(empty, METRICS, '2026-11-02')).toBe('Dimanche 25 octobre : pas de données')
    expect(cellLabel(empty, METRICS.slice(0, 1), '2026-11-02')).toBe('Dimanche 25 octobre : cafés : pas de données')
  })

  it('gardent le compteur global même à zéro, et des chiffres courts', () => {
    expect(errorCountText(0)).toBe('Aucun enregistrement en erreur')
    expect(errorCountText(1)).toBe('1 enregistrement en erreur')
    expect(compact(312)).toBe('312')
    expect(compact(1000)).toBe('1 k')
    expect(compact(1240)).toBe('1,2 k')
  })
})

describe('les unités de Garmin', () => {
  const metric = (label: string, unit: string) => ({ metric: label, label, unit })
  const nbsp = (text: string) => text.replace(/\u202f/g, ' ')

  it('dit des pas, des kcal, une durée de sommeil et des séances', () => {
    expect(nbsp(valueText(metric('Pas', 'pas'), 8412))).toBe('8 412 pas')
    expect(nbsp(valueText(metric('Calories brûlées', 'kcal'), 2350))).toBe('2 350 kcal')
    expect(valueText(metric('Sommeil', 'min'), 432)).toBe('7 h 12')
    expect(valueText(metric('Sommeil', 'min'), 45)).toBe('45 min')
    expect(valueText(metric('Séances', 'séance'), 1)).toBe('1 séance')
    expect(valueText(metric('Séances', 'séance'), 2)).toBe('2 séances')
    expect(valueText(metric('Pas', 'pas'), 0)).toBe('0 pas')
    expect(valueText(metric('Pas', 'pas'), null)).toBe('pas : pas de données')
    expect(valueText(metric('Vidéos YouTube', 'vidéo'), 1)).toBe('1 vidéo YouTube')
    expect(valueText(metric('Vidéos YouTube', 'vidéo'), 3)).toBe('3 vidéos YouTube')
    expect(valueText(metric('Vidéos YouTube', 'vidéo'), null)).toBe('vidéos YouTube : pas de données')
  })

  it('met le sommeil en heures dans une case', () => {
    expect(cellText(metric('Sommeil', 'min'), 432)).toBe('7,2 h')
    expect(cellText(metric('Sommeil', 'min'), 420)).toBe('7 h')
    expect(cellText(metric('Sommeil', 'min'), 40)).toBe('40')
    expect(cellText(metric('Pas', 'pas'), 8412)).toBe('8,4 k')
    expect(hoursText(60)).toBe('1 h 00')
  })

  it('nomme la source Garmin', () => {
    expect(sourceName('garmin')).toBe('Garmin')
    expect(sourceName('garmin-2')).toBe('Garmin')
  })
})
