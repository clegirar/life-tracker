import { describe, expect, it } from 'vitest'
import { masonrySpan } from './useMasonry'

describe('la maçonnerie de la page du jour', () => {
  it('couvre la hauteur de la carte plus l’écart, arrondie au pixel supérieur', () => {
    expect(masonrySpan(300, 20)).toBe(320)
    expect(masonrySpan(300.2, 20)).toBe(321)
    expect(masonrySpan(0, 0)).toBe(1)
  })
})
