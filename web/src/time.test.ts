import { describe, expect, it } from 'vitest'
import { addDays, dayTitle, longDate, parisDate, parisInstant, parisTime, toRfc3339 } from './time'

const rfc = (date: string, time: string) => {
  const instant = parisInstant(date, time)
  return instant === null ? null : toRfc3339(instant)
}

describe('parisInstant', () => {
  it('donne le décalage de la saison', () => {
    expect(rfc('2026-07-14', '08:15')).toBe('2026-07-14T08:15:00+02:00')
    expect(rfc('2026-01-14', '08:15')).toBe('2026-01-14T08:15:00+01:00')
  })

  it("prend le premier passage d'une heure qui existe deux fois", () => {
    expect(rfc('2026-10-25', '02:30')).toBe('2026-10-25T02:30:00+02:00')
    expect(parisInstant('2026-10-25', '02:30')).toBe(Date.parse('2026-10-25T00:30:00Z'))
    expect(rfc('2026-10-25', '02:00')).toBe('2026-10-25T02:00:00+02:00')
    expect(rfc('2026-10-25', '02:59')).toBe('2026-10-25T02:59:00+02:00')
  })

  it('reste juste autour du passage à l’heure d’hiver', () => {
    expect(rfc('2026-10-25', '00:30')).toBe('2026-10-25T00:30:00+02:00')
    expect(rfc('2026-10-25', '01:59')).toBe('2026-10-25T01:59:00+02:00')
    expect(rfc('2026-10-25', '03:00')).toBe('2026-10-25T03:00:00+01:00')
    expect(rfc('2026-10-25', '23:30')).toBe('2026-10-25T23:30:00+01:00')
  })

  it("refuse une heure qui n'existe pas", () => {
    expect(parisInstant('2026-03-29', '02:00')).toBeNull()
    expect(parisInstant('2026-03-29', '02:30')).toBeNull()
    expect(parisInstant('2026-03-29', '02:59')).toBeNull()
    expect(rfc('2026-03-29', '01:59')).toBe('2026-03-29T01:59:00+01:00')
    expect(rfc('2026-03-29', '03:00')).toBe('2026-03-29T03:00:00+02:00')
  })

  it('rattache minuit au bon jour', () => {
    expect(rfc('2026-10-31', '00:00')).toBe('2026-10-31T00:00:00+01:00')
    expect(parisDate(parisInstant('2026-10-31', '23:59')!)).toBe('2026-10-31')
  })
})

describe('parisDate et parisTime', () => {
  it('découpent en heure de Paris, pas en UTC ni en heure de l’appareil', () => {
    const lateUtc = Date.parse('2026-10-24T22:30:00Z')
    expect(parisDate(lateUtc)).toBe('2026-10-25')
    expect(parisTime(lateUtc)).toBe('00:30')
    const winter = Date.parse('2026-10-25T22:30:00Z')
    expect(parisDate(winter)).toBe('2026-10-25')
    expect(parisTime(winter)).toBe('23:30')
  })

  it('les deux passages de 02:30 ont des décalages différents', () => {
    expect(toRfc3339(Date.parse('2026-10-25T00:30:00Z'))).toBe('2026-10-25T02:30:00+02:00')
    expect(toRfc3339(Date.parse('2026-10-25T01:30:00Z'))).toBe('2026-10-25T02:30:00+01:00')
  })

  it('un instant actuel garde ses secondes', () => {
    expect(toRfc3339(Date.parse('2026-07-14T06:15:42.900Z'))).toBe('2026-07-14T08:15:42+02:00')
  })
})

describe('calendrier', () => {
  it('addDays traverse les 25 h et les mois', () => {
    expect(addDays('2026-10-25', 1)).toBe('2026-10-26')
    expect(addDays('2026-10-25', -1)).toBe('2026-10-24')
    expect(addDays('2026-12-31', 1)).toBe('2027-01-01')
    expect(addDays('2026-03-29', 1)).toBe('2026-03-30')
  })

  it('nomme les jours', () => {
    expect(longDate('2026-10-31', '2026-10-31')).toBe('Samedi 31 octobre')
    expect(longDate('2025-10-31', '2026-10-31')).toBe('Vendredi 31 octobre 2025')
    expect(dayTitle('2026-10-31', '2026-10-31')).toBe("Aujourd'hui")
    expect(dayTitle('2026-10-30', '2026-10-31')).toBe('Hier')
    expect(dayTitle('2026-10-29', '2026-10-31')).toBe('Jeudi')
  })
})
