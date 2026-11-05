import { describe, expect, it } from 'vitest'
import type { DayVideo, SourcesHealth, VideoWatch } from './api'
import { channelOfDay, newChannels, thumbnailUrl, totalText, videoEmptyText, videoUrl, watchRows, youtubeSilent } from './video'

const watch = (overrides: Partial<VideoWatch>): VideoWatch => ({
  time: '20:30',
  occurred_at: '2026-10-14T18:30:00.123Z',
  video_id: 'vidFictif01',
  title: 'Première vidéo fictive',
  channel_id: 'UCchaineFictiveA000000',
  channel_name: 'Chaîne Fictive A',
  duration_s: 549,
  ...overrides,
})

const DAY: DayVideo = {
  date: '2026-10-14',
  count: 3,
  seconds: 549 + 3723,
  without_duration: 1,
  watches: [
    watch({}),
    watch({
      time: '21:00',
      occurred_at: '2026-10-14T19:00:00Z',
      video_id: 'vidFictif02',
      title: 'Deuxième vidéo fictive',
      channel_id: 'UCchaineFictiveB000000',
      channel_name: 'Chaîne Fictive B',
      duration_s: 3723,
    }),
    watch({ time: '22:00', occurred_at: '2026-10-14T20:00:00Z', video_id: 'vidSupprime', title: null, channel_id: null, channel_name: null, duration_s: null }),
  ],
  discoveries: [
    { level: 'channel', key: 'UCchaineFictiveB000000', name: 'Chaîne Fictive B' },
    { level: 'video', key: 'vidFictif02', name: 'Deuxième vidéo fictive' },
    { level: 'video', key: 'vidSupprime', name: 'vidSupprime' },
  ],
}

function health(overrides: { silent?: boolean; coverage?: [string, string] | null; connected?: boolean } = {}): SourcesHealth {
  const { silent = false, coverage = ['2026-10-01T10:00:00Z', '2026-10-14T20:00:00Z'], connected = true } = overrides
  return {
    sources: connected ? [{ name: 'youtube', last_seen_at: null, silence_threshold_seconds: 259_200, silent }] : [],
    metrics: [
      {
        metric: 'video.watch',
        first_event_at: coverage?.[0] ?? null,
        last_event_at: coverage?.[1] ?? null,
        freshness_threshold_seconds: 259_200,
        stale: false,
        coverage: coverage ? [{ source: 'youtube', first_at: coverage[0], last_at: coverage[1] }] : [],
      },
    ],
    errors: 0,
  }
}

describe('totalText', () => {
  it('ne confond pas zéro et inconnu', () => {
    expect(totalText(null)).toBe('–')
    expect(totalText(0)).toBe('0 min')
  })
  it('arrondit à la minute et passe aux heures', () => {
    expect(totalText(9)).toBe("moins d'1 min")
    expect(totalText(549)).toBe('9 min')
    expect(totalText(549 + 3723)).toBe('1 h 11')
  })
})

describe('watchRows', () => {
  const rows = watchRows(DAY)
  it('relie chaque vidéo à sa page, sans rien intégrer', () => {
    expect(rows[0].url).toBe('https://www.youtube.com/watch?v=vidFictif01')
    expect(videoUrl('a&b')).toBe('https://www.youtube.com/watch?v=a%26b')
    expect(rows[0].channel).toBe('Chaîne Fictive A')
    expect(rows[1].length).toBe('1:02:03')
    expect(rows[0].length).toBe('9:09')
  })
  it('une vidéo supprimée : ni titre, ni lien, ni miniature, ni chaîne, ni durée', () => {
    expect(rows[2]).toMatchObject({ title: null, url: null, thumbnail: null, channel: null, length: null })
  })
  it('la miniature de i.ytimg.com, pour un identifiant valide seulement', () => {
    expect(rows[0].thumbnail).toBe('https://i.ytimg.com/vi/vidFictif01/mqdefault.jpg')
    expect(thumbnailUrl('Ab_9-xYz012')).toBe('https://i.ytimg.com/vi/Ab_9-xYz012/mqdefault.jpg')
    for (const id of ['', 'court', 'vidFictif012', 'vidFictif0/', '../../x/abc', 'vid%2FFicti', 'vidFictif0\n']) expect(thumbnailUrl(id)).toBeNull()
    expect(watchRows({ ...DAY, watches: [watch({ video_id: 'a&b' })] })[0]).toMatchObject({ thumbnail: null, url: 'https://www.youtube.com/watch?v=a%26b' })
  })
  it('distingue une vidéo déjà vue avant ce jour', () => {
    expect(rows.map((row) => row.seenBefore)).toEqual([true, false, false])
  })
})

describe('channelOfDay', () => {
  it('à partir de deux vidéos de la même chaîne', () => {
    expect(channelOfDay(DAY.watches)).toBeNull()
    const again = watch({ occurred_at: '2026-10-14T21:00:00Z' })
    expect(channelOfDay([...DAY.watches, again])).toEqual({ name: 'Chaîne Fictive A', count: 2 })
  })
})

describe('newChannels', () => {
  it('garde les chaînes seules', () => {
    expect(newChannels(DAY).map((d) => d.name)).toEqual(['Chaîne Fictive B'])
  })
})

describe('videoEmptyText', () => {
  const today = '2026-10-20'
  it('sans source, ou avant son premier export', () => {
    expect(videoEmptyText('2026-10-14', today, null)).toBe('Aucune vidéo reçue pour ce jour.')
    expect(videoEmptyText('2026-10-14', today, health({ connected: false }))).toBe("YouTube n'est pas encore relié.")
    expect(videoEmptyText('2026-10-14', today, health({ coverage: null }))).toMatch(/^Pas encore de données YouTube/)
  })
  it("avant l'historique : pas de données", () => {
    expect(videoEmptyText('2026-09-30', today, health())).toBe('Pas de données YouTube à cette date.')
  })
  it("après la dernière vue reçue : on ne sait pas encore, l'export est quotidien", () => {
    expect(videoEmptyText('2026-10-19', today, health())).toMatch(/^Pas encore de données pour ce jour/)
    expect(videoEmptyText(today, today, health())).toMatch(/^Pas encore de données pour ce jour/)
  })
  it("dans l'historique : aucune vidéo", () => {
    expect(videoEmptyText('2026-10-10', today, health())).toBe('Aucune vidéo ce jour-là.')
    expect(videoEmptyText('2026-10-14', '2026-10-15', health())).toBe('Aucune vidéo hier.')
    expect(videoEmptyText('2026-10-14', '2026-10-14', health())).toBe("Aucune vidéo aujourd'hui.")
  })
})

describe('youtubeSilent', () => {
  it('suit la source YouTube seule', () => {
    expect(youtubeSilent(null)).toBe(false)
    expect(youtubeSilent(health())).toBe(false)
    expect(youtubeSilent(health({ silent: true }))).toBe(true)
  })
})
