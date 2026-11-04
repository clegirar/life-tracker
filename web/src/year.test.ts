import { describe, expect, it } from 'vitest'
import type { MonthDay, MonthMetric } from './api'
import { monthGrid } from './calendar'
import { heatLevel, heatThresholds, monthSummary, periodThresholds, yearMonths } from './year'

const COFFEE: MonthMetric = { metric: 'coffee', label: 'Cafés', unit: 'tasse' }
const day = (date: string, coffee: number | null): MonthDay => ({
  date,
  values: { coffee },
  errors: 0,
  pending: [],
  to_check: [],
})

describe("la grille de l'année", () => {
  it('compte douze mois, de janvier à décembre', () => {
    const months = yearMonths('2026')
    expect(months).toHaveLength(12)
    expect(months[0]).toBe('2026-01')
    expect(months[11]).toBe('2026-12')
    // Chaque mini-mois reprend la grille du mois : lundi d'abord, 365 jours en tout.
    expect(months.flatMap((m) => monthGrid(m).flat().filter(Boolean))).toHaveLength(365)
    expect(yearMonths('2028').flatMap((m) => monthGrid(m).flat().filter(Boolean))).toHaveLength(366)
  })
})

describe('les paliers', () => {
  it('pas de données, zéro et quatre paliers ne se confondent jamais', () => {
    const t = heatThresholds([null, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9])
    expect(t).toEqual([3, 5, 7, 9])
    expect(heatLevel(null, t)).toBe('none')
    expect(heatLevel(0, t)).toBe(0)
    expect(heatLevel(1, t)).toBe(1)
    expect(heatLevel(3, t)).toBe(1)
    expect(heatLevel(4, t)).toBe(2)
    expect(heatLevel(6, t)).toBe(3)
    expect(heatLevel(8, t)).toBe(4)
    expect(heatLevel(9, t)).toBe(4)
  })

  it('les zéros et les absences ne comptent pas dans les quartiles', () => {
    expect(heatThresholds([0, 0, null, 0, 4, 4])).toEqual([4, 4, 4, 4])
    expect(heatThresholds([null, 0])).toEqual([])
  })

  it('une année uniforme est au plus foncé, le maximum aussi', () => {
    const t = heatThresholds([2, 2, 2])
    expect(heatLevel(2, t)).toBe(4)
    // Peu de valeurs distinctes : le moins est pâle, le plus est foncé.
    const few = heatThresholds([1, 1, 2, 2, 2, 3, 3])
    expect(heatLevel(1, few)).toBe(1)
    expect(heatLevel(2, few)).toBe(2)
    expect(heatLevel(3, few)).toBe(4)
  })

  it('une valeur sans seuil (aucune donnée positive) reste un zéro', () => {
    expect(heatLevel(0, [])).toBe(0)
  })
})

describe('les paliers de l’année affichée', () => {
  it('ses jours passés, ses valeurs non nulles', () => {
    const days = [day('2026-11-01', 1), day('2026-11-02', 0), day('2026-11-03', 3), day('2026-11-04', 9)]
    // Le 4 est futur : il ne compte pas, même s'il portait une valeur.
    expect(periodThresholds(days, 'coffee', '2026-11-03')).toEqual([1.5, 2, 2.5, 3])
    expect(heatLevel(3, periodThresholds(days, 'coffee', '2026-11-03'))).toBe(4)
    expect(periodThresholds(days, 'music.listen', '2026-11-03')).toEqual([])
  })
})

describe('le résumé lu par un lecteur d’écran', () => {
  it('dit le total et les jours sans données, jamais le futur', () => {
    const days = [day('2026-11-01', null), day('2026-11-02', 2), day('2026-11-03', 0), day('2026-11-04', null)]
    expect(monthSummary('2026-11', days, [COFFEE], '2026-11-03')).toBe('Novembre 2026 : 2 cafés, 1 jour sans données')
    expect(monthSummary('2026-11', days.slice(1), [COFFEE], '2026-11-03')).toBe('Novembre 2026 : 2 cafés')
    expect(monthSummary('2026-11', [day('2026-11-01', null)], [COFFEE], '2026-11-03')).toBe('Novembre 2026 : pas de données')
    expect(monthSummary('2026-12', [day('2026-12-01', null)], [COFFEE], '2026-11-03')).toBe('Décembre 2026')
    // Plusieurs métriques : une partie chacune.
    const LISTENS: MonthMetric = { metric: 'music.listen', label: 'Écoutes', unit: 'écoute' }
    expect(monthSummary('2026-11', days.slice(1, 3), [COFFEE, LISTENS], '2026-11-03')).toBe('Novembre 2026 : 2 cafés ; écoutes : pas de données')
  })
})
