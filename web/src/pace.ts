// La trace d'une séance en couleurs d'allure, comme sur Garmin Connect : la vitesse de chaque tronçon
// (distance haversine sur le temps écoulé), lissée sur une centaine de mètres, puis rangée en classes
// sur une échelle propre à la séance, bornée aux centiles 10 et 90 pour qu'un arrêt ou un sprint
// n'écrase pas le reste. Fonctionne sur les traces déjà au journal : rien ne change au backend.

import { distanceM, type LatLng } from './location'

/** Le nombre de classes d'allure, de la plus lente (0) à la plus rapide. */
export const PACE_CLASSES = 6

/** La longueur sur laquelle une vitesse se lisse : le tronçon et ses voisins, à 50 m de part et d'autre. */
const SMOOTHING_M = 100

/** Un morceau de trace d'une même classe : ses points, bout à bout avec le morceau suivant. */
export interface PaceRun {
  level: number
  points: LatLng[]
}

export interface PaceDrawing {
  runs: PaceRun[]
  /** Les bornes de l'échelle, en m/s : la classe 0 commence à `slow`, la dernière finit à `fast`. */
  slow: number
  fast: number
}

/**
 * Le centile `p` (0 à 1) d'un ensemble pondéré : la plus petite valeur dont le poids cumulé atteint
 * `p` du poids total. Pondéré par la distance : un long tronçon de la trace pèse ce qu'il occupe.
 */
function weightedPercentile(values: { value: number; weight: number }[], p: number): number {
  const sorted = [...values].sort((a, b) => a.value - b.value)
  const total = sorted.reduce((sum, item) => sum + item.weight, 0)
  let cumulated = 0
  for (const item of sorted) {
    cumulated += item.weight
    if (cumulated >= p * total) return item.value
  }
  return sorted[sorted.length - 1].value
}

/**
 * Les vitesses lissées de chaque tronçon (`null` sans durée), en m/s. Chaque tronçon prend la distance
 * totale sur la durée totale des tronçons dont le milieu est à moins de 50 m du sien : un arrêt reste
 * visible, un point bruité se fond dans ses voisins.
 */
export function smoothedSpeeds(points: [number, number, number][]): (number | null)[] {
  const n = points.length - 1
  const length: number[] = []
  const duration: number[] = []
  const middle: number[] = []
  let along = 0
  for (let i = 0; i < n; i++) {
    const d = distanceM([points[i][0], points[i][1]], [points[i + 1][0], points[i + 1][1]])
    length.push(d)
    duration.push(points[i + 1][2] - points[i][2])
    middle.push(along + d / 2)
    along += d
  }
  const speeds: (number | null)[] = []
  let from = 0
  let to = 0
  let meters = 0
  let seconds = 0
  for (let i = 0; i < n; i++) {
    // Une fenêtre glissante : on ajoute devant, on retire derrière.
    while (to < n && middle[to] - middle[i] <= SMOOTHING_M / 2) {
      meters += length[to]
      seconds += duration[to]
      to++
    }
    while (middle[i] - middle[from] > SMOOTHING_M / 2) {
      meters -= length[from]
      seconds -= duration[from]
      from++
    }
    speeds.push(seconds > 0 ? meters / seconds : null)
  }
  return speeds
}

/**
 * La trace découpée par classe d'allure, ou `null` quand l'allure ne se lit pas : moins de deux points,
 * des secondes absentes ou figées, ou une allure si régulière que l'échelle n'aurait pas de sens. On
 * dessine alors la trace d'une seule couleur.
 */
export function paceDrawing(points: [number, number, number][], classes = PACE_CLASSES): PaceDrawing | null {
  if (points.length < 2) return null
  const speeds = smoothedSpeeds(points)
  const weighted: { value: number; weight: number }[] = []
  for (let i = 0; i < speeds.length; i++) {
    const speed = speeds[i]
    const weight = distanceM([points[i][0], points[i][1]], [points[i + 1][0], points[i + 1][1]])
    if (speed !== null && weight > 0) weighted.push({ value: speed, weight })
  }
  if (weighted.length < 2) return null
  const slow = weightedPercentile(weighted, 0.1)
  const fast = weightedPercentile(weighted, 0.9)
  // Moins de 2 % d'écart entre les bornes : une seule couleur dirait mieux la régularité.
  if (!(fast > 0) || fast - slow < fast * 0.02) return null

  const levelOf = (speed: number) => Math.min(classes - 1, Math.max(0, Math.floor(((speed - slow) / (fast - slow)) * classes)))
  const runs: PaceRun[] = []
  let previous = 0
  for (let i = 0; i < speeds.length; i++) {
    // Un tronçon sans durée prend la classe du précédent : il n'interrompt pas la trace.
    const speed = speeds[i]
    const level = speed === null ? previous : levelOf(speed)
    const start: LatLng = [points[i][0], points[i][1]]
    const end: LatLng = [points[i + 1][0], points[i + 1][1]]
    const last = runs[runs.length - 1]
    if (last && last.level === level) last.points.push(end)
    else runs.push({ level, points: [start, end] })
    previous = level
  }
  return { runs, slow, fast }
}
