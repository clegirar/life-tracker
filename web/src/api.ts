// Tout appel au backend : même origine, cookie de session, en-tête anti-CSRF.

export interface Cup {
  time: string
  occurred_at: string
  source: string
  key: string
}

export interface Day {
  date: string
  count: number
  cups: Cup[]
}

export interface OutgoingRecord {
  key: string
  kind: 'coffee.cup' | 'record.cancel'
  schema_version: 1
  occurred_at: string
  payload: Record<string, string>
}

/** La session n'est plus valable. */
export class Unauthorized extends Error {}
/** Le réseau ou le serveur ont failli : la même requête peut réussir plus tard. */
export class Transient extends Error {}
/** Le serveur refuse : renvoyer la même requête échouerait pareil. */
export class Refused extends Error {}

const TIMEOUT_MS = 10_000

async function call(path: string, init: RequestInit = {}): Promise<Response> {
  let response: Response
  try {
    response = await fetch(path, {
      ...init,
      credentials: 'same-origin',
      signal: AbortSignal.timeout(TIMEOUT_MS),
      headers: {
        'Content-Type': 'application/json',
        'X-CSRF-Protection': '1',
        'X-Client-Version': `web-${__APP_VERSION__}`,
        ...init.headers,
      },
    })
  } catch {
    throw new Transient()
  }
  if (response.status >= 500) throw new Transient()
  return response
}

export async function login(username: string, password: string): Promise<boolean> {
  const response = await call('/v1/auth/login', {
    method: 'POST',
    body: JSON.stringify({ username, password }),
  })
  if (response.status === 401) return false
  if (!response.ok) throw new Refused()
  return true
}

export async function logout(): Promise<void> {
  await call('/v1/auth/logout', { method: 'POST' })
}

export async function fetchDay(date: string): Promise<Day> {
  const response = await call(`/v1/days/${date}`)
  if (response.status === 401) throw new Unauthorized()
  if (!response.ok) throw new Refused()
  return response.json()
}

/** Accepté ou déjà reçu : c'est écrit au journal. Sinon, une exception dit quoi faire. */
export async function sendRecord(record: OutgoingRecord): Promise<void> {
  const response = await call('/v1/records', {
    method: 'POST',
    body: JSON.stringify({ records: [record] }),
  })
  if (response.status === 401) throw new Unauthorized()
  if (!response.ok) throw new Refused()
  const body: { results: { status: string }[] } = await response.json()
  if (body.results[0]?.status === 'rejected') throw new Refused()
}

export interface MusicRow {
  name: string
  artist: string
  listens: number
}

export interface MusicArtist {
  name: string
  tracks: number
  listens: number
}

export interface DayMusic {
  date: string
  listens: number
  /** `null` : aucune durée connue, ce qui n'est pas zéro minute. */
  minutes: number | null
  listens_without_duration: number
  artist_of_day: { name: string; listens: number } | null
  tracks: MusicRow[]
  artists: MusicArtist[]
  albums: MusicRow[]
}

export interface SourceHealth {
  name: string
  last_seen_at: string | null
  silence_threshold_seconds: number
  silent: boolean
}

export interface MetricHealth {
  metric: string
  first_event_at: string | null
  last_event_at: string | null
  freshness_threshold_seconds: number
  stale: boolean
}

export interface SourcesHealth {
  sources: SourceHealth[]
  metrics: MetricHealth[]
}

async function read<T>(path: string): Promise<T> {
  const response = await call(path)
  if (response.status === 401) throw new Unauthorized()
  if (!response.ok) throw new Refused()
  return response.json()
}

export const fetchDayMusic = (date: string) => read<DayMusic>(`/v1/days/${date}/music`)
export const fetchSourcesHealth = () => read<SourcesHealth>('/v1/sources/health')
