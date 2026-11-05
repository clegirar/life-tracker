import { describe, expect, it } from 'vitest'
import { parseRoute, routePath, TODAY } from './route'

const TODAY_DATE = '2026-11-03'

describe('les adresses', () => {
  it('lit chaque page', () => {
    expect(parseRoute('/', TODAY_DATE)).toEqual(TODAY)
    expect(parseRoute('/jour/2026-10-25', TODAY_DATE)).toEqual({ page: 'day', date: '2026-10-25' })
    expect(parseRoute('/mois/2026-02', TODAY_DATE)).toEqual({ page: 'month', month: '2026-02' })
    expect(parseRoute('/annee/2022', TODAY_DATE)).toEqual({ page: 'year', year: '2022' })
    expect(parseRoute('/etat', TODAY_DATE)).toEqual({ page: 'status' })
    // Une barre finale ne change rien.
    expect(parseRoute('/mois/2026-02/', TODAY_DATE)).toEqual({ page: 'month', month: '2026-02' })
  })

  it("le jour d'aujourd'hui suit aujourd'hui", () => {
    expect(parseRoute(`/jour/${TODAY_DATE}`, TODAY_DATE)).toEqual(TODAY)
  })

  it('refuse une adresse invalide ou future', () => {
    for (const path of [
      '/jour/2026-11-04',
      '/jour/2026-02-30',
      '/jour/2026-1-01',
      '/jour/hier',
      '/mois/2026-12',
      '/mois/2026-13',
      '/mois/2026-00',
      '/annee/2027',
      '/annee/26',
      '/jour',
      '/inconnu',
      '/jour/2026-10-25/autre',
    ]) {
      expect(parseRoute(path, TODAY_DATE), path).toBeNull()
    }
  })

  it('écrit chaque page, et relit ce qu’elle écrit', () => {
    for (const route of [TODAY, { page: 'day', date: '2026-10-25' }, { page: 'month', month: '2026-10' }, { page: 'year', year: '2026' }, { page: 'status' }] as const) {
      expect(parseRoute(routePath(route), TODAY_DATE)).toEqual(route)
    }
    expect(routePath(TODAY)).toBe('/')
  })
})
