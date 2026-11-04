import { describe, expect, it } from 'vitest'
import type { DayActivity, WorkoutDetails } from './api'
import { clockText, isEmpty, kmText, paceText, sleepSpan, sleepStages, workoutFigures, workoutType } from './activity'

const nbsp = (text: string) => text.replace(/[\u202f\u00a0]/g, ' ')

const run: WorkoutDetails = {
  activity_id: 1,
  type: 'running',
  duration_s: 1802.4,
  moving_s: 1780,
  distance_m: 5012.3,
  avg_speed_mps: 2.78,
  avg_hr: 151,
  max_hr: 176,
  calories: 389,
  location_name: 'Ville Fictive',
}

describe('la carte Activité', () => {
  it('traduit les types connus et garde la clé des autres', () => {
    expect(workoutType('running')).toBe('Course')
    expect(workoutType('walking')).toBe('Marche')
    expect(workoutType('cycling')).toBe('Vélo')
    expect(workoutType('kitesurfing')).toBe('kitesurfing')
  })

  it('donne distance, durée, allure et cardio d’une course', () => {
    expect(workoutFigures(run).map((f) => `${f.label} ${nbsp(f.value)}`)).toEqual([
      'Distance 5,01 km',
      'Durée 29:40',
      'Allure 6:00 /km',
      'FC moyenne 151 bpm',
      'FC max 176 bpm',
    ])
    expect(paceText(run)).toBe('6:00 /km')
    expect(paceText({ ...run, type: 'cycling', avg_speed_mps: 6.75 })).toBe('24,3 km/h')
    expect(paceText({ ...run, avg_speed_mps: null })).toBeNull()
    expect(workoutFigures({ ...run, distance_m: null, avg_speed_mps: null, avg_hr: null, max_hr: null, moving_s: null })).toEqual([
      { label: 'Durée', value: '30:02' },
    ])
  })

  it('écrit les durées comme un chrono et les distances en km', () => {
    expect(clockText(59)).toBe('0:59')
    expect(clockText(3725)).toBe('1:02:05')
    expect(nbsp(kmText(12345))).toBe('12,35 km')
  })

  it('range les phases de la nuit et dit son début et sa fin à Paris', () => {
    const details = {
      start: '2026-11-02T21:41:00Z',
      end: '2026-11-03T05:05:00Z',
      deep_seconds: 4920,
      light_seconds: 14400,
      rem_seconds: null,
      awake_seconds: 0,
    }
    expect(sleepStages(details).map((s) => [s.key, s.minutes])).toEqual([
      ['deep', 82],
      ['light', 240],
    ])
    expect(sleepSpan(details)).toBe('22:41 – 06:05')
    expect(sleepSpan({ ...details, end: null })).toBeNull()
  })

  it('ne confond pas une journée sans données avec une journée sans séance', () => {
    const empty: DayActivity = { date: '2026-11-03', steps: null, calories: null, sleep: null, workouts: [] }
    expect(isEmpty(empty)).toBe(true)
    expect(isEmpty({ ...empty, steps: { value: 0, details: { distance_m: 0 } } })).toBe(false)
  })
})
