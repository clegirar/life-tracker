import { describe, expect, it } from 'vitest'
import type { DayLocations, LocationPlace, LocationWorkout, SourcesHealth } from './api'
import {
  dayRows,
  distanceM,
  distanceText,
  drawingPoints,
  durationText,
  hasTrace,
  hereText,
  iphoneSilent,
  locationKind,
  mapDrawing,
  mapLabel,
  placesCount,
  stillText,
  totalLengthM,
  traceLengthM,
  uncoveredText,
  placeRows,
  workoutRows,
  workoutsDetail,
} from './location'

// Des coordonnées fictives, en mer au large de Marseille.
const TODAY = '2026-11-05'
const DATE = '2026-10-20'
const NBSP = ' '

function place(overrides: Partial<LocationPlace> = {}): LocationPlace {
  return {
    origin: 'visit',
    lat: 43.201,
    lon: 5.285,
    arrival: '2026-10-20T08:12:00Z',
    departure: '2026-10-20T08:48:00Z',
    arrival_time: '10:12',
    departure_time: '10:48',
    arrived_before: false,
    departed_after: false,
    duration_s: 2160,
    ongoing: false,
    horizontal_accuracy: 35,
    ...overrides,
  }
}

function workout(overrides: Partial<LocationWorkout> = {}): LocationWorkout {
  return {
    activity_id: 4242,
    activity_type: 'running',
    start: '2026-10-20T09:00:00Z',
    end: '2026-10-20T09:45:00Z',
    start_time: '11:00',
    end_time: '11:45',
    distance_m: 8200,
    points: [
      [43.25, 5.3],
      [43.26, 5.31],
      [43.27, 5.3],
    ],
    ...overrides,
  }
}

function day(overrides: Partial<DayLocations> = {}): DayLocations {
  return {
    date: DATE,
    covered: true,
    point_count: 3,
    segments: [
      {
        start_time: '10:00',
        end_time: '10:10',
        start: '2026-10-20T08:00:00Z',
        end: '2026-10-20T08:10:00Z',
        points: [
          [43.2, 5.28],
          [43.2, 5.29],
        ],
      },
    ],
    places: [place()],
    workouts: [],
    ...overrides,
  }
}

function health(sources: Partial<SourcesHealth['sources'][number]>[]): SourcesHealth {
  return {
    sources: sources.map((source) => ({ name: 'iphone', last_seen_at: null, silence_threshold_seconds: 172_800, silent: false, ...source })),
    metrics: [],
    errors: 0,
  }
}

describe('le jour', () => {
  it('distingue un jour sans données, un jour sans déplacement et un jour avec', () => {
    expect(locationKind(day({ covered: false, segments: [], places: [] }))).toBe('uncovered')
    expect(locationKind(day({ segments: [], places: [], point_count: 0 }))).toBe('still')
    expect(locationKind(day())).toBe('moved')
    expect(locationKind(day({ segments: [] }))).toBe('moved')
  })

  it('dit un jour suivi sans déplacement, sans parler de données', () => {
    expect(stillText(TODAY, TODAY)).toBe("Aucun déplacement enregistré pour l'instant aujourd'hui.")
    expect(stillText('2026-11-04', TODAY)).toBe('Aucun déplacement enregistré hier.')
    expect(stillText(DATE, TODAY)).toBe('Aucun déplacement enregistré ce jour-là.')
  })

  it('dit un jour sans données selon le dernier envoi de l’iPhone, jamais « aucun déplacement »', () => {
    expect(uncoveredText(DATE, TODAY, null)).toBe('Aucune position reçue pour ce jour.')
    expect(uncoveredText(DATE, TODAY, health([]))).toBe("La position de l'iPhone n'est pas encore reliée.")
    const seen = health([{ last_seen_at: '2026-11-03T18:00:00Z' }])
    expect(uncoveredText(DATE, TODAY, seen)).toBe('Pas de données de position à cette date.')
    expect(uncoveredText('2026-11-03', TODAY, seen)).toBe('Pas encore de position reçue pour ce jour.')
    expect(uncoveredText('2026-11-04', TODAY, seen)).toBe("Pas encore de données pour ce jour : l'iPhone n'a rien envoyé depuis.")
    expect(uncoveredText(TODAY, TODAY, seen)).toBe("Pas encore de position reçue aujourd'hui.")
    expect(uncoveredText(TODAY, TODAY, health([{ last_seen_at: null }]))).toBe("Pas encore de position reçue aujourd'hui.")
    for (const text of [uncoveredText(DATE, TODAY, seen), uncoveredText('2026-11-04', TODAY, seen)]) expect(text).not.toMatch(/déplacement/)
  })

  it('suit le silence de la source, remplacement de token compris', () => {
    expect(iphoneSilent(null)).toBe(false)
    expect(iphoneSilent(health([{ silent: false }]))).toBe(false)
    expect(iphoneSilent(health([{ name: 'iphone-2', silent: true }]))).toBe(true)
    expect(iphoneSilent(health([{ name: 'iphonex', silent: true }]))).toBe(false)
  })
})

describe('les durées et les distances', () => {
  it('dit une durée en minutes, puis en heures', () => {
    expect(durationText(20)).toBe("moins d'1 min")
    expect(durationText(2160)).toBe('36 min')
    expect(durationText(7500)).toBe('2 h 05')
  })

  it('mesure la trace morceau par morceau, sans relier les morceaux', () => {
    // 0,01° de longitude à 43,2° de latitude : environ 810 m.
    expect(distanceM([43.2, 5.28], [43.2, 5.29])).toBeCloseTo(810, -1)
    const twoPieces = day({
      segments: [
        { ...day().segments[0], points: [[43.2, 5.28], [43.2, 5.29]] },
        { ...day().segments[0], points: [[43.3, 5.28]] },
        { ...day().segments[0], points: [[43.3, 5.28], [43.3, 5.29]] },
      ],
    })
    expect(traceLengthM(twoPieces)).toBeCloseTo(810 + 809, -1)
    expect(hasTrace(twoPieces)).toBe(true)
    expect(hasTrace(day({ segments: [{ ...day().segments[0], points: [[43.2, 5.28]] }] }))).toBe(false)
  })

  it('dit une distance en mètres sous le kilomètre, puis en kilomètres', () => {
    expect(distanceText(0)).toBe('0 m')
    expect(distanceText(812)).toBe('810 m')
    expect(distanceText(996)).toBe('1,0 km')
    expect(distanceText(12_430)).toBe('12,4 km')
  })
})

describe('les lieux', () => {
  it('les classe par arrivée, numérotés comme les épingles', () => {
    const later = place({ arrival: '2026-10-20T12:00:00Z', arrival_time: '14:00', departure_time: '15:30', duration_s: 5400, lat: 43.21 })
    const rows = placeRows(day({ places: [later, place()] }), TODAY)
    expect(rows.map((row) => [row.number, row.range, row.duration, row.note])).toEqual([
      [1, '10:12 – 10:48', '36 min', null],
      [2, '14:00 – 15:30', '1 h 30', null],
    ])
    expect(rows[1].at).toEqual([43.21, 5.285])
  })

  it('dit un lieu commencé la veille ou fini le lendemain, ou plus loin', () => {
    const night = place({
      arrival: '2026-10-19T20:10:00Z',
      departure: '2026-10-21T06:45:00Z',
      arrival_time: '22:10',
      departure_time: '08:45',
      arrived_before: true,
      departed_after: true,
      duration_s: 124_200,
    })
    const note = (v: LocationPlace) => placeRows(day({ places: [v] }), TODAY)[0].note
    expect(note(night)).toBe("Depuis la veille, jusqu'au lendemain")
    expect(note({ ...night, departed_after: false })).toBe('Depuis la veille')
    expect(note({ ...night, arrived_before: false })).toBe("Jusqu'au lendemain")
    expect(note({ ...night, arrival: '2026-10-17T20:10:00Z', departed_after: false })).toBe(`Depuis le 17${NBSP}oct.`)
    expect(note({ ...night, arrived_before: false, departure: '2026-10-23T06:45:00Z' })).toBe(`Jusqu'au 23${NBSP}oct.`)
    expect(placeRows(day({ places: [night] }), TODAY)[0].duration).toBe('34 h 30')
  })

  it('dit un séjour détecté comme une visite, et le lieu où je suis encore', () => {
    const stay = place({ origin: 'points', horizontal_accuracy: null, arrival: '2026-10-20T06:00:00Z', arrival_time: '08:00', departure_time: '10:05', duration_s: 7500 })
    const here = place({
      origin: 'points',
      arrival: '2026-10-20T15:14:00Z',
      arrival_time: '17:14',
      departure: null,
      departure_time: null,
      duration_s: 4320,
      ongoing: true,
      horizontal_accuracy: null,
    })
    const rows = placeRows(day({ places: [here, stay] }), TODAY)
    expect(rows.map((row) => [row.number, row.range, row.duration, row.note, row.ongoing])).toEqual([
      [1, '08:00 – 10:05', '2 h 05', null, false],
      [2, 'depuis 17:14', 'en cours', null, true],
    ])
    expect(rows[1].arrival).toBe(Date.parse('2026-10-20T15:14:00Z'))
    // Arrivé la veille et encore là : la veille seulement, jamais de lendemain.
    expect(placeRows(day({ places: [{ ...here, arrived_before: true, arrival: '2026-10-19T20:10:00Z' }] }), TODAY)[0].note).toBe('Depuis la veille')
  })

  it('dit depuis quand je suis ici, et combien de temps, de minute en minute', () => {
    const arrival = Date.parse('2026-11-05T16:14:40Z') // 17 h 14 à Paris
    expect(hereText(arrival, Date.parse('2026-11-05T17:26:10Z'))).toEqual({ since: `Ici depuis 17${NBSP}h${NBSP}14`, elapsed: '1 h 12' })
    expect(hereText(arrival, Date.parse('2026-11-05T16:14:59Z')).elapsed).toBe("moins d'1 min")
    expect(hereText(arrival, Date.parse('2026-11-05T16:52:00Z')).elapsed).toBe('38 min')
    expect(hereText(arrival, Date.parse('2026-11-06T07:00:00Z'))).toEqual({ since: `Ici depuis hier à 17${NBSP}h${NBSP}14`, elapsed: '14 h 46' })
    expect(hereText(arrival, Date.parse('2026-11-08T07:00:00Z')).since).toBe(`Ici depuis le 5${NBSP}nov. à 17${NBSP}h${NBSP}14`)
  })
})

describe('le nombre de lieux', () => {
  it('compte par arrivée, comme le calendrier, et dit à part ceux de la veille', () => {
    expect(placesCount(day())).toEqual({ arrived: 1, earlier: null })
    const night = place({ arrived_before: true })
    expect(placesCount(day({ places: [night, place(), place()] }))).toEqual({ arrived: 2, earlier: 'et 1 depuis la veille' })
    expect(placesCount(day({ places: [night] }))).toEqual({ arrived: 0, earlier: 'et 1 depuis la veille' })
  })
})

describe('la carte', () => {
  it('dessine les morceaux, les points isolés et les épingles', () => {
    const single = { ...day().segments[0], points: [[43.25, 5.3]] as [number, number][] }
    const drawing = mapDrawing(day({ segments: [day().segments[0], single] }), TODAY)!
    expect(drawing.lines).toEqual([[[43.2, 5.28], [43.2, 5.29]]])
    expect(drawing.dots).toEqual([[43.25, 5.3]])
    expect(drawing.pins).toEqual([{ at: [43.201, 5.285], label: '1', current: false }])
    expect(drawingPoints(drawing)).toHaveLength(4)
  })

  it('réunit deux lieux au même endroit dans une épingle', () => {
    const evening = place({ arrival: '2026-10-20T16:00:00Z', lat: 43.2012 })
    const far = place({ arrival: '2026-10-20T12:00:00Z', lat: 43.21 })
    expect(mapDrawing(day({ places: [evening, far, place()] }), TODAY)!.pins).toEqual([
      { at: [43.201, 5.285], label: '1 · 3', current: false },
      { at: [43.21, 5.285], label: '2', current: false },
    ])
  })

  it('met en évidence l’épingle du lieu où je suis encore', () => {
    const here = place({ arrival: '2026-10-20T16:00:00Z', lat: 43.2012, departure: null, departure_time: null, ongoing: true })
    const far = place({ arrival: '2026-10-20T12:00:00Z', lat: 43.21 })
    expect(mapDrawing(day({ places: [here, far, place()] }), TODAY)!.pins).toEqual([
      { at: [43.201, 5.285], label: '1 · 3', current: true },
      { at: [43.21, 5.285], label: '2', current: false },
    ])
  })

  it('ne dessine rien sans trace ni lieu', () => {
    expect(mapDrawing(day({ segments: [], places: [] }), TODAY)).toBeNull()
  })

  it('se décrit pour un lecteur d’écran', () => {
    expect(mapLabel(day())).toBe('Carte des déplacements : 1 lieu, 810 m de trace')
    expect(mapLabel(day({ places: [] }))).toBe('Carte des déplacements : aucun lieu, 810 m de trace')
    expect(mapLabel(day({ segments: [], places: [place(), place()] }))).toBe('Carte des déplacements : 2 lieux')
    const here = place({ arrival: '2026-10-20T16:00:00Z', departure: null, departure_time: null, ongoing: true })
    expect(mapLabel(day({ places: [place(), here] }))).toBe('Carte des déplacements : 2 lieux, 810 m de trace, lieu 2 en cours')
  })
})

describe('les séances de la montre', () => {
  it('comptent dans la distance du jour, par le chiffre de Garmin ou, sans lui, par leur trace', () => {
    expect(totalLengthM(day({ workouts: [workout()] }))).toBeCloseTo(traceLengthM(day()) + 8200, 6)
    const traced = workout({ distance_m: null })
    const along = distanceM([43.25, 5.3], [43.26, 5.31]) + distanceM([43.26, 5.31], [43.27, 5.3])
    expect(totalLengthM(day({ segments: [], workouts: [traced] }))).toBeCloseTo(along, 6)
    expect(hasTrace(day({ segments: [], workouts: [workout()] }))).toBe(true)
  })

  it('se disent sous la distance, par type, dans l’ordre du jour', () => {
    expect(workoutsDetail(day())).toBeNull()
    expect(workoutsDetail(day({ workouts: [workout()] }))).toBe('dont 8,2 km de course')
    const ride = workout({ activity_id: 2, activity_type: 'cycling', distance_m: 12_000 })
    const again = workout({ activity_id: 3, distance_m: 1800 })
    expect(workoutsDetail(day({ workouts: [workout(), ride, again] }))).toBe('dont 10,0 km de course et 12,0 km de vélo')
    const walk = workout({ activity_id: 4, activity_type: 'walking', distance_m: 950 })
    expect(workoutsDetail(day({ workouts: [workout(), ride, walk] }))).toBe('dont 8,2 km de course, 12,0 km de vélo et 950 m de marche')
    expect(workoutsDetail(day({ workouts: [workout({ distance_m: null })] }))).toBe('dont 2,8 km de course')
  })

  it('prennent leur place dans la liste, à leur heure, sans changer les numéros des lieux', () => {
    const after = place({ arrival: '2026-10-20T09:45:00Z', departure: '2026-10-20T11:00:00Z', arrival_time: '11:45', departure_time: '13:00' })
    const rows = dayRows(day({ places: [after, place()], workouts: [workout()] }), TODAY)
    expect(rows.map((entry) => entry.kind)).toEqual(['place', 'workout', 'place'])
    expect(rows.flatMap((entry) => (entry.kind === 'place' ? [entry.row.number] : []))).toEqual([1, 2])
    expect(rows[1].row).toMatchObject({ label: 'Course', range: '11:00 – 11:45', figure: '8,2 km' })
  })

  it('disent leur durée sans distance ni trace', () => {
    const still = workout({ distance_m: null, points: [[43.25, 5.3]] })
    expect(workoutRows(day({ workouts: [still] }))[0].figure).toBe('45 min')
  })

  it('font d’un jour sans trace ni lieu, voire sans données de l’iPhone, un jour avec déplacements', () => {
    expect(locationKind(day({ segments: [], places: [], workouts: [workout()] }))).toBe('moved')
    expect(locationKind(day({ covered: false, segments: [], places: [], workouts: [workout()] }))).toBe('moved')
  })

  it('se dessinent sur la carte et en élargissent le cadre', () => {
    const drawing = mapDrawing(day({ workouts: [workout(), workout({ activity_id: 2, points: [[43.3, 5.3]] })] }), TODAY)!
    expect(drawing.workouts).toEqual([workout().points])
    expect(drawingPoints(drawing)).toHaveLength(6)
    expect(mapDrawing(day({ segments: [], places: [], workouts: [workout()] }), TODAY)!.workouts).toHaveLength(1)
  })

  it('se comptent pour un lecteur d’écran', () => {
    expect(mapLabel(day({ workouts: [workout()] }))).toBe('Carte des déplacements : 1 lieu, 810 m de trace, 1 séance')
    expect(mapLabel(day({ segments: [], places: [], workouts: [workout(), workout({ activity_id: 2 })] }))).toBe(
      'Carte des déplacements : aucun lieu, 2 séances',
    )
  })
})
