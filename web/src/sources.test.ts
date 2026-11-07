import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import type { SourceStatus } from './api'
import { allKeys, frenchSpaces, keyExpiry, OTHER_KEYS, sheetState, sourceOf, TRACKERS, type KeyInfo } from './sources'

// Le 5 nov. 2026, 14 h 00 à Paris (heure d'hiver, UTC+1).
const NOW = Date.parse('2026-11-05T13:00:00Z')
const NBSP = '\u00a0'
const DAY = 86_400_000
const iso = (offsetMs: number) => new Date(NOW + offsetMs).toISOString()

const repo = (path: string) => readFileSync(new URL(`../../${path}`, import.meta.url), 'utf8')

function source(overrides: Partial<SourceStatus> = {}): SourceStatus {
  return {
    name: 'garmin',
    label: 'Garmin Connect',
    last_seen_at: iso(-10 * 60_000),
    silence_threshold_seconds: 43_200,
    silent: false,
    status: { state: 'ok', reason: null, expires_at: iso(300 * DAY), next_run_at: null },
    status_reported_at: iso(-10 * 60_000),
    metrics: [],
    authorization_url: null,
    ...overrides,
  }
}

const sheet = (id: string) => TRACKERS.find((tracker) => tracker.id === id)!
const liveKey = (id: string) => sheet(id).keys.find((key) => key.live)!
const texts = (key: KeyInfo) => [key.name, key.expiry]

describe('les fiches', () => {
  it('couvrent chaque tracker, chacun avec au moins une clé', () => {
    expect(TRACKERS.map((tracker) => tracker.id)).toEqual(['cafe', 'musique', 'garmin', 'youtube', 'twitch', 'steam', 'iphone'])
    for (const tracker of TRACKERS) {
      expect(tracker.arrival, tracker.id).not.toBe('')
      expect(tracker.keys.length, tracker.id).toBeGreaterThan(0)
    }
    expect(OTHER_KEYS.every((group) => group.keys.length > 0)).toBe(true)
  })

  it('donnent à chaque clé un nom unique et une échéance', () => {
    const names = allKeys().map((key) => key.name)
    expect(new Set(names).size).toBe(names.length)
    for (const key of allKeys()) for (const text of texts(key)) expect(text.trim(), key.name).not.toBe('')
  })

  it("ne citent que des variables qu'utilisent vraiment le dépôt", () => {
    const corpus = [repo('docs/deploiement.md'), repo('.env.prod.example'), repo('.github/workflows/release.yml'), repo('extension/sign.sh'), repo('CLAUDE.md')].join('\n')
    for (const key of allKeys()) {
      for (const variable of texts(key).join(' ').match(/\b[A-Z][A-Z0-9]*_[A-Z0-9_]+\b/g) ?? []) {
        expect(corpus, `${key.name} : ${variable}`).toContain(variable)
      }
    }
  })

  it("ne contiennent aucune valeur : ni nom d'hôte, ni adresse, ni jeton", () => {
    const titles = [...TRACKERS.flatMap((tracker) => [tracker.title, tracker.arrival]), ...OTHER_KEYS.map((group) => group.title)]
    const all = [...titles, ...allKeys().flatMap(texts)].join('\n')
    expect(all).not.toMatch(/ts\.net/)
    expect(all).not.toMatch(/\b100\.\d+\.\d+\.\d+\b/)
    expect(all).not.toMatch(/[0-9a-f]{24,}/i)
    expect(all).not.toMatch(/[A-Za-z0-9_-]{40,}/)
  })
})

describe('les échéances', () => {
  it('reprennent la clé Garmin que rapporte le serveur, avec les seuils de la page État', () => {
    const key = liveKey('garmin')
    expect(keyExpiry(key, source(), NOW)).toEqual({ text: `Valable jusqu'au 1${NBSP}sept. 2027.`, urgency: null, live: true })
    const soon = source({ status: { state: 'ok', reason: null, expires_at: iso(7 * DAY), next_run_at: null } })
    expect(keyExpiry(key, soon, NOW)).toEqual({ text: `Expire le 12${NBSP}nov., dans 7 jours.`, urgency: 'soon', live: true })
    const past = source({ status: { state: 'action_required', reason: 'key_rejected', expires_at: iso(-2 * DAY), next_run_at: null } })
    expect(keyExpiry(key, past, NOW)).toEqual({ text: `Expirée le 3${NBSP}nov.`, urgency: 'past', live: true })
  })

  it("gardent le texte figé sans source ou sans échéance rapportée", () => {
    const key = liveKey('garmin')
    expect(keyExpiry(key, null, NOW)).toEqual({ text: key.expiry, urgency: null, live: false })
    expect(keyExpiry(key, source({ status: null }), NOW).live).toBe(false)
    // Une clé sans échéance au serveur ne lit pas celle de la source.
    const token = sheet('garmin').keys.find((candidate) => !candidate.live)!
    expect(keyExpiry(token, source({ status: { state: 'ok', reason: null, expires_at: iso(-DAY), next_run_at: null } }), NOW).urgency).toBeNull()
  })

  it("disent l'accès Google à l'heure près, et l'accord à redonner avant tout", () => {
    const key = liveKey('youtube')
    const youtube = (status: SourceStatus['status']) => source({ name: 'youtube', label: 'YouTube', status })
    expect(keyExpiry(key, youtube({ state: 'ok', reason: null, expires_at: iso(5 * DAY), next_run_at: null }), NOW).urgency).toBeNull()
    expect(keyExpiry(key, youtube({ state: 'ok', reason: null, expires_at: '2026-11-06T19:00:00Z', next_run_at: null }), NOW)).toEqual({
      text: `Expire demain à 20${NBSP}h${NBSP}00.`,
      urgency: 'soon',
      live: true,
    })
    expect(keyExpiry(key, youtube({ state: 'action_required', reason: 'consent_expired', expires_at: null, next_run_at: null }), NOW)).toEqual({
      text: 'Accord expiré : à redonner depuis la page État.',
      urgency: 'past',
      live: true,
    })
    expect(keyExpiry(key, youtube({ state: 'action_required', reason: 'consent_required', expires_at: null, next_run_at: null }), NOW).urgency).toBe('soon')
  })
})

describe('les sources des fiches', () => {
  it('trouvent la source active, même recréée', () => {
    const data = { sources: [source({ name: 'garmin-2' }), source({ name: 'lastfm', label: 'Musique' })] }
    expect(sourceOf(sheet('garmin'), data)?.name).toBe('garmin-2')
    expect(sourceOf(sheet('musique'), data)?.name).toBe('lastfm')
    expect(sourceOf(sheet('youtube'), data)).toBeNull()
    expect(sourceOf(sheet('cafe'), null)).toBeNull()
  })

  it("prennent l'état de la carte d'état", () => {
    expect(sheetState(null, NOW)).toBeNull()
    expect(sheetState(source(), NOW)).toBe('ok')
    expect(sheetState(source({ status: { state: 'error', reason: 'service_unavailable', expires_at: null, next_run_at: null } }), NOW)).toBe('broken')
  })

  it('gardent les signes doubles avec leur mot', () => {
    expect(frenchSpaces('« Tester » ; puis : fini')).toBe(`«${NBSP}Tester${NBSP}»${NBSP}; puis${NBSP}: fini`)
  })
})
