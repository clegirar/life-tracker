import { describe, expect, it } from 'vitest'
import { paceDrawing, PACE_CLASSES, smoothedSpeeds } from './pace'

// Vers l'est, sur l'équateur : 0,0001° de longitude font environ 11,1 m.
const STEP_M = 11.119
const east = (i: number, sec: number): [number, number, number] => [0, i * 0.0001, sec]

/** Une trace fictive : des tronçons de 11 m, chacun parcouru en `seconds[i]` secondes. */
function track(seconds: number[]): [number, number, number][] {
  const points = [east(0, 0)]
  let t = 0
  seconds.forEach((s, i) => {
    t += s
    points.push(east(i + 1, t))
  })
  return points
}

describe('l’allure d’une séance', () => {
  it('donne la vitesse de chaque tronçon, lissée sur une centaine de mètres', () => {
    // Une allure régulière : la même vitesse partout.
    const steady = smoothedSpeeds(track(Array(20).fill(4)))
    expect(steady).toHaveLength(20)
    for (const speed of steady) expect(speed).toBeCloseTo(STEP_M / 4, 2)
    // Un tronçon isolé plus lent se fond dans ses voisins, sans disparaître.
    const bump = smoothedSpeeds(track([...Array(10).fill(4), 40, ...Array(10).fill(4)]))
    expect(bump[10]!).toBeGreaterThan(STEP_M / 40)
    expect(bump[10]!).toBeLessThan(STEP_M / 4)
    expect(bump[0]).toBeCloseTo(STEP_M / 4, 2)
  })

  it('range la trace en classes, de lent à rapide, par morceaux bout à bout', () => {
    // 300 m lents puis 300 m rapides : les deux bouts de l'échelle.
    const drawing = paceDrawing(track([...Array(27).fill(6), ...Array(27).fill(3)]))!
    expect(drawing).not.toBeNull()
    expect(drawing.slow).toBeCloseTo(STEP_M / 6, 2)
    expect(drawing.fast).toBeCloseTo(STEP_M / 3, 2)
    expect(drawing.runs[0].level).toBe(0)
    expect(drawing.runs[drawing.runs.length - 1].level).toBe(PACE_CLASSES - 1)
    // Chaque morceau reprend le dernier point du précédent : la trace reste continue.
    for (let i = 1; i < drawing.runs.length; i++) {
      expect(drawing.runs[i].points[0]).toEqual(drawing.runs[i - 1].points[drawing.runs[i - 1].points.length - 1])
    }
    expect(drawing.runs.reduce((sum, run) => sum + run.points.length - 1, 0)).toBe(54)
    for (const run of drawing.runs) {
      expect(run.level).toBeGreaterThanOrEqual(0)
      expect(run.level).toBeLessThan(PACE_CLASSES)
    }
  })

  it('borne l’échelle aux centiles 10 et 90 : un arrêt ne l’écrase pas', () => {
    const seconds = Array.from({ length: 100 }, (_, i) => 3 + (i % 10) / 10)
    seconds[50] = 600 // dix minutes à l'arrêt sur un tronçon
    const drawing = paceDrawing(track(seconds))!
    expect(drawing.slow).toBeGreaterThan(STEP_M / 10)
  })

  it('renonce aux couleurs sans secondes, sans écart ni assez de points', () => {
    expect(paceDrawing(track(Array(20).fill(0)))).toBeNull()
    expect(paceDrawing(track(Array(20).fill(4)))).toBeNull()
    expect(paceDrawing([east(0, 0)])).toBeNull()
    expect(paceDrawing([])).toBeNull()
  })

  it('garde un tronçon sans durée dans la classe du précédent', () => {
    const seconds = [...Array(27).fill(6), 0, ...Array(27).fill(3)]
    // Le lissage couvre ce tronçon avec ses voisins : la trace reste entière, sans trou.
    const drawing = paceDrawing(track(seconds))!
    expect(drawing.runs.reduce((sum, run) => sum + run.points.length - 1, 0)).toBe(55)
  })
})
