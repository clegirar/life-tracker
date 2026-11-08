import { describe, expect, it } from 'vitest'
import { Refused, Transient, Unauthorized } from './api'
import { keys, retryTransient } from './queries'

describe('les relectures', () => {
  it('une panne passagère se retente deux fois, jamais un refus ni une session perdue', () => {
    expect(retryTransient(0, new Transient())).toBe(true)
    expect(retryTransient(1, new Transient())).toBe(true)
    expect(retryTransient(2, new Transient())).toBe(false)
    expect(retryTransient(0, new Unauthorized())).toBe(false)
    expect(retryTransient(0, new Refused())).toBe(false)
  })

  it("les lectures d'un jour vivent sous sa clé", () => {
    expect(keys.dayPart('music', '2026-11-08').slice(0, 2)).toEqual([...keys.day('2026-11-08')])
  })
})
