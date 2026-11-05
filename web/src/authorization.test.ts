import { describe, expect, it } from 'vitest'
import { callbackFrom, OUTCOME_TEXT, outcomeOf, storedOutcome } from './authorization'

const STATE = 'abcdefghijklmnopqrstuvwxyz0123456789-._~ABCDEFG'

describe('callbackFrom', () => {
  it('garde exactement l’état et le code, ou l’état et l’erreur', () => {
    expect(callbackFrom(`?state=${STATE}&code=4%2F0AbCd&scope=x`)).toEqual({ state: STATE, code: '4/0AbCd' })
    expect(callbackFrom(`?error=access_denied&state=${STATE}`)).toEqual({ state: STATE, error: 'access_denied' })
  })
  it('sans réponse lisible : rien à transmettre', () => {
    for (const search of ['', '?code=x', `?state=${STATE}`, `?state=${STATE}&code=x&error=y`, '?state=&code=x']) {
      expect(callbackFrom(search), search).toBeNull()
    }
  })
})

describe('outcomeOf', () => {
  it('distingue un accord, un refus et une erreur de Google', () => {
    expect(outcomeOf({ state: STATE, code: 'c' }, 'received')).toBe('received')
    expect(outcomeOf({ state: STATE, error: 'access_denied' }, 'received')).toBe('refused')
    expect(outcomeOf({ state: STATE, error: 'invalid_scope' }, 'received')).toBe('google_error')
    expect(outcomeOf({ state: STATE, code: 'c' }, 'expired')).toBe('expired')
  })
  it('les textes', () => {
    expect(`${OUTCOME_TEXT.received.title} ${OUTCOME_TEXT.received.text}`).toBe('Accord reçu. La collecte YouTube reprend dans une minute.')
    expect(OUTCOME_TEXT.expired).toEqual({ title: 'Ce lien a expiré.', text: 'Relance depuis la page État.' })
    expect(OUTCOME_TEXT.refused.title).toBe("Tu as refusé l'accord.")
  })
})

describe('storedOutcome', () => {
  it('ne relit qu’une issue connue', () => {
    expect(storedOutcome({ authorization: 'received' })).toBe('received')
    expect(storedOutcome({ authorization: 'autre' })).toBeNull()
    expect(storedOutcome(null)).toBeNull()
  })
})
