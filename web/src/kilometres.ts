// Les repères kilométriques d'une séance, comme sur Garmin Connect : un repère à chaque kilomètre de la
// trace, à sa position exacte entre les deux points qui l'entourent. Sur une longue séance, l'écart grandit
// (2, 5, 10 km…) pour ne jamais dépasser une vingtaine de repères sur la carte.

import { distanceM, type LatLng } from './location'

/** Au plus autant de repères sur une trace : au-delà, ils se chevauchent et couvrent le tracé. */
export const MAX_KILOMETRE_MARKERS = 20

/** Les écarts possibles entre deux repères, en km, du plus fin au plus large. */
const STEPS_KM = [1, 2, 5, 10, 20, 50, 100]

/** L'écart entre deux repères pour une trace de `lengthM` mètres : le plus fin qui en donne au plus 20. */
export function kilometreStep(lengthM: number): number {
  return STEPS_KM.find((step) => Math.floor(lengthM / 1000 / step) <= MAX_KILOMETRE_MARKERS) ?? STEPS_KM[STEPS_KM.length - 1]
}

export interface KilometreMarker {
  /** Le kilomètre atteint : 1, 2, 3… ou 2, 4, 6… */
  km: number
  at: LatLng
}

/**
 * Les repères d'une trace (`[lat, lon, sec]`) : un à chaque multiple de l'écart, interpolé entre les deux
 * points qui l'entourent. Aucun sous 1 km. La distance suit la trace, en haversine, comme l'allure.
 */
export function kilometreMarkers(points: [number, number, number][]): KilometreMarker[] {
  const lengths: number[] = []
  let total = 0
  for (let i = 1; i < points.length; i++) {
    const d = distanceM([points[i - 1][0], points[i - 1][1]], [points[i][0], points[i][1]])
    lengths.push(d)
    total += d
  }
  const step = kilometreStep(total)
  const markers: KilometreMarker[] = []
  let along = 0
  let next = step
  for (let i = 0; i < lengths.length; i++) {
    const d = lengths[i]
    while (d > 0 && next * 1000 <= along + d) {
      const t = (next * 1000 - along) / d
      const [lat1, lon1] = points[i]
      const [lat2, lon2] = points[i + 1]
      markers.push({ km: next, at: [lat1 + (lat2 - lat1) * t, lon1 + (lon2 - lon1) * t] })
      next += step
    }
    along += d
  }
  return markers
}
