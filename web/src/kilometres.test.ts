import { describe, expect, it } from 'vitest'
import { kilometreMarkers, kilometreStep, MAX_KILOMETRE_MARKERS } from './kilometres'
import { distanceM } from './location'

// Vers l'est, sur l'équateur : 0,001° de longitude font environ 111,2 m.
const STEP_M = distanceM([0, 0], [0, 0.001])

/** Une trace fictive vers l'est, de `count` tronçons d'environ 111 m, une seconde chacun. */
function track(count: number, degrees = 0.001): [number, number, number][] {
  return Array.from({ length: count + 1 }, (_, i) => [0, i * degrees, i])
}

describe('les repères kilométriques d’une séance', () => {
  it('pose un repère à chaque kilomètre, à sa position exacte entre deux points', () => {
    // 50 tronçons de 111 m : 5,56 km, donc cinq repères.
    const markers = kilometreMarkers(track(50))
    expect(markers.map((marker) => marker.km)).toEqual([1, 2, 3, 4, 5])
    for (const marker of markers) {
      expect(marker.at[0]).toBe(0)
      // La longitude interpolée tombe exactement à N km du départ, le long de la trace.
      expect(distanceM([0, 0], marker.at)).toBeCloseTo(marker.km * 1000, 3)
    }
    // Le premier kilomètre (8 × 111 m = 890 m, 9 × 111 m = 1 001 m) tombe entre deux points, sur aucun.
    expect(markers[0].at[1]).toBeGreaterThan(0.008)
    expect(markers[0].at[1]).toBeLessThan(0.009)
    expect(markers[0].at[1]).toBeCloseTo(1000 / STEP_M / 1000, 6)
  })

  it('n’en pose aucun sous 1 km, ni sans tronçon', () => {
    expect(kilometreMarkers(track(8))).toEqual([]) // environ 890 m
    expect(kilometreMarkers(track(0))).toEqual([])
    expect(kilometreMarkers([])).toEqual([])
  })

  it('pose un repère sur un point qui tombe pile au kilomètre, et ignore un tronçon sans longueur', () => {
    const points: [number, number, number][] = [[0, 0, 0], [0, 0, 1], ...track(20).slice(1)]
    const markers = kilometreMarkers(points)
    expect(markers.map((marker) => marker.km)).toEqual([1, 2])
    // Une trace dont un point est exactement au kilomètre : le repère est sur ce point.
    const exact = kilometreMarkers([[0, 0, 0], [0, 1000 / STEP_M / 1000, 1], [0, 0.015, 2]])
    expect(exact).toHaveLength(1)
    expect(distanceM([0, 0], exact[0].at)).toBeCloseTo(1000, 3)
  })

  it('élargit l’écart sur une longue séance : 1, 2, 5 puis 10 km, jamais plus de 20 repères', () => {
    expect(kilometreStep(999)).toBe(1)
    expect(kilometreStep(20_999)).toBe(1)
    expect(kilometreStep(21_000)).toBe(2)
    expect(kilometreStep(41_999)).toBe(2)
    expect(kilometreStep(42_195)).toBe(5)
    expect(kilometreStep(100_000)).toBe(5)
    expect(kilometreStep(105_000)).toBe(10)
    expect(kilometreStep(209_999)).toBe(10)
    expect(kilometreStep(210_000)).toBe(20)
    for (let km = 0; km <= 2_000; km += 7) {
      expect(Math.floor(km / kilometreStep(km * 1000))).toBeLessThanOrEqual(MAX_KILOMETRE_MARKERS)
    }
    // Un marathon : un repère tous les 5 km, numéroté par son kilomètre.
    const marathon = kilometreMarkers(track(380)) // environ 42,25 km
    expect(marathon.map((marker) => marker.km)).toEqual([5, 10, 15, 20, 25, 30, 35, 40])
  })
})
