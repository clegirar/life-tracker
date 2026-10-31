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
