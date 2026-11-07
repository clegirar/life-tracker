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
  kind: 'coffee.cup' | 'record.cancel' | 'music.mark_known'
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

/** Accepté ou déjà reçu : c'est écrit au journal, en une seule transaction. Sinon, une exception dit quoi faire. */
export async function sendRecords(records: OutgoingRecord[]): Promise<void> {
  const response = await call('/v1/records', {
    method: 'POST',
    body: JSON.stringify({ records }),
  })
  if (response.status === 401) throw new Unauthorized()
  if (!response.ok) throw new Refused()
  const body: { results: { status: string }[] } = await response.json()
  if (body.results.some((result) => result.status === 'rejected')) throw new Refused()
}

export const sendRecord = (record: OutgoingRecord) => sendRecords([record])

/** `crypto.randomUUID` n'existe qu'en contexte sécurisé, or le téléphone en dev passe par http. */
export function newKey(): string {
  if (crypto.randomUUID) return crypto.randomUUID()
  const bytes = crypto.getRandomValues(new Uint8Array(16))
  bytes[6] = (bytes[6] & 0x0f) | 0x40
  bytes[8] = (bytes[8] & 0x3f) | 0x80
  const hex = Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('')
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`
}

export interface MusicRow {
  name: string
  /** `null` : un titre de l'historique dont l'artiste n'est pas attribuable. */
  artist: string | null
  listens: number
}

export interface MusicArtist {
  name: string
  tracks: number
  listens: number
}

export interface KnownMark {
  source: string
  key: string
}

/** Un élément écouté pour la première fois ce jour-là : une découverte tant qu'il n'est pas marqué. */
export interface FirstListen {
  level: 'artist' | 'album' | 'track'
  item_key: string
  name: string
  artist: string
  known_marks: KnownMark[]
}

export interface DayMusic {
  date: string
  listens: number
  /** `null` : aucune durée connue, ce qui n'est pas zéro minute. */
  minutes: number | null
  listens_without_duration: number
  /** Écoutes de l'historique Apple sans artiste attribuable : aucun n'est inventé. */
  listens_without_artist: number
  artist_of_day: { name: string; listens: number } | null
  tracks: MusicRow[]
  artists: MusicArtist[]
  albums: MusicRow[]
  discoveries: number
  first_listens: FirstListen[]
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
  /** Ce que chaque source couvre, de la plus ancienne à la plus récente. */
  coverage: { source: string; first_at: string; last_at: string }[]
}

export interface SourcesHealth {
  sources: SourceHealth[]
  metrics: MetricHealth[]
  /** Le compteur global : enregistrements en erreur d'interprétation. */
  errors: number
}

export interface MonthMetric {
  metric: string
  label: string
  unit: string | null
}

/** Un jour de la vue du mois. */
export interface MonthDay {
  date: string
  /** Par métrique : l'agrégat du jour, ou `null` quand il n'y a pas de données, ce qui n'est pas zéro. */
  values: Record<string, number | null>
  /** Enregistrements de ce jour en erreur : la journée est incomplète. */
  errors: number
  /** Sources muettes qui n'ont pas encore couvert ce jour. */
  pending: string[]
  /** Métriques sans nouvelle donnée depuis leur seuil de fraîcheur. */
  to_check: string[]
}

export interface Month {
  month: string
  today: string
  metrics: MonthMetric[]
  days: MonthDay[]
}

/** L'année, jour par jour, de la même forme que le mois. */
export interface Year {
  year: number
  today: string
  metrics: MonthMetric[]
  days: MonthDay[]
}

async function read<T>(path: string): Promise<T> {
  const response = await call(path)
  if (response.status === 401) throw new Unauthorized()
  if (!response.ok) throw new Refused()
  return response.json()
}

export const fetchDayMusic = (date: string) => read<DayMusic>(`/v1/days/${date}/music`)
export const fetchMonth = (month: string) => read<Month>(`/v1/months/${month}`)
export const fetchYear = (year: string) => read<Year>(`/v1/years/${year}`)
export const fetchSourcesHealth = () => read<SourcesHealth>('/v1/sources/health')

/** Une mesure Garmin du jour : sa valeur et ses détails, tels que l'interprète les a écrits. */
export interface Measure<D> {
  value: number | null
  details: D
}

export interface StepsDetails {
  distance_m: number | null
}

export interface CaloriesDetails {
  active: number | null
  resting: number | null
}

export interface SleepDetails {
  start: string | null
  end: string | null
  deep_seconds: number | null
  light_seconds: number | null
  rem_seconds: number | null
  awake_seconds: number | null
  /** Fréquence cardiaque de la nuit, absente sans échantillons (nuits d'avant, montre muette). */
  lowest_hr?: number | null
  avg_hr?: number | null
}

export interface WorkoutDetails {
  activity_id: number
  /** La clé de Garmin : `running`, `walking`, `cycling`… */
  type: string
  duration_s: number | null
  moving_s: number | null
  distance_m: number | null
  avg_speed_mps: number | null
  avg_hr: number | null
  max_hr: number | null
  calories: number | null
  location_name: string | null
}

export interface Workout {
  /** Heure de Paris du début. */
  time: string
  occurred_at: string
  details: WorkoutDetails
  has_track: boolean
}

/** `null` : pas de données ce jour-là (montre pas portée), jamais zéro. */
export interface DayActivity {
  date: string
  steps: Measure<StepsDetails> | null
  calories: Measure<CaloriesDetails> | null
  sleep: Measure<SleepDetails> | null
  workouts: Workout[]
}

export interface Track {
  activity_id: number
  /** `[lat, lon, secondes depuis le début]`. */
  points: [number, number, number][]
}

export const fetchDayActivity = (date: string) => read<DayActivity>(`/v1/days/${date}/activity`)
export const fetchTrack = (activityId: number) => read<Track>(`/v1/activities/${activityId}/track`)

/** L'état rapporté par un connecteur, en vocabulaire fermé : la web app le traduit en phrases. */
export interface ReportedStatus {
  state: 'ok' | 'action_required' | 'error'
  reason:
    | 'key_rejected'
    | 'consent_required'
    | 'consent_expired'
    | 'profile_private'
    | 'rate_limited'
    | 'service_unavailable'
    | 'ingest_failed'
    | null
  /** L'échéance de la clé ou du jeton. */
  expires_at: string | null
  next_run_at: string | null
  /** La dernière collecte réussie (pour YouTube, le dernier export) ; absente d'un état plus ancien. */
  last_collected_at?: string | null
}

/** Une source de la page d'état. */
export interface SourceStatus {
  name: string
  /** Le nom affiché ; `null`, la page montre `name`. */
  label: string | null
  /** Le dernier contact réussi. */
  last_seen_at: string | null
  /** `null` : jamais muette (le raccourci café). */
  silence_threshold_seconds: number | null
  silent: boolean
  status: ReportedStatus | null
  status_reported_at: string | null
  /** Pour chaque métrique que la source alimente, sa dernière donnée. */
  metrics: { metric: string; label: string; last_at: string }[]
  /** La page de consentement qu'un connecteur attend que j'ouvre ; `null` hors d'une demande en cours. */
  authorization_url: string | null
}

export interface SourcesStatus {
  sources: SourceStatus[]
}

export const fetchSourcesStatus = () => read<SourcesStatus>('/v1/sources/status')

/** Une vidéo vue. `title` à `null` : une vidéo supprimée ou privée, dont seul l'id reste. */
export interface VideoWatch {
  /** Heure de Paris. */
  time: string
  occurred_at: string
  video_id: string
  title: string | null
  channel_id: string | null
  channel_name: string | null
  /** La durée de la vidéo, pas le temps regardé ; `null` : inconnue. */
  duration_s: number | null
}

/** Une chaîne ou une vidéo vue pour la première fois ce jour-là. */
export interface VideoDiscovery {
  level: 'channel' | 'video'
  key: string
  name: string
}

export interface DayVideo {
  date: string
  count: number
  /** La somme des durées connues ; `null` : aucune connue, ce qui n'est pas zéro. */
  seconds: number | null
  without_duration: number
  watches: VideoWatch[]
  discoveries: VideoDiscovery[]
}

export const fetchDayVideo = (date: string) => read<DayVideo>(`/v1/days/${date}/video`)

/** Ce que Google a renvoyé sur `/autorisation`, transmis au connecteur par le backend. */
export type AuthorizationCallback = { state: string; code: string } | { state: string; error: string }

/** `received` : remis au connecteur. `expired` : aucune demande n'attend cet état. `invalid` : refusé tel quel. */
export type CallbackResult = 'received' | 'expired' | 'invalid'

export async function sendAuthorizationCallback(callback: AuthorizationCallback): Promise<CallbackResult> {
  const response = await call('/v1/authorizations/callback', {
    method: 'POST',
    body: JSON.stringify(callback),
  })
  if (response.status === 401) throw new Unauthorized()
  if (response.status === 204) return 'received'
  if (response.status === 404) return 'expired'
  return 'invalid'
}

/** Une chaîne Twitch regardée ce jour-là. */
export interface StreamChannel {
  /** Le login, pour le lien `https://www.twitch.tv/<login>`. */
  key: string
  name: string
  /** L'union de ses segments ce jour-là. */
  minutes: number
  /** `live`, `vod`, `clip`, dans cet ordre. */
  contents: string[]
  /** Heure de Paris du premier début, qui peut tomber la veille. */
  first_time: string
  /** Heure de Paris de la dernière fin, qui peut tomber le lendemain. */
  last_time: string
}

/** Une chaîne regardée pour la première fois ce jour-là. */
export interface StreamDiscovery {
  level: 'channel'
  key: string
  name: string
}

export interface DayStream {
  date: string
  /** L'union de tous les segments : deux streams à la fois comptent une fois. */
  minutes: number
  /** La somme des temps par chaîne : `summed_minutes - minutes`, le temps à deux streams. */
  summed_minutes: number
  /** Les segments qui touchent ce jour-là ; 0 : rien de reçu. */
  segments: number
  /** Par temps décroissant, puis par nom. */
  channels: StreamChannel[]
  /** Le temps des segments dont l'extension n'a pas su la chaîne. */
  unknown_minutes: number
  discoveries: StreamDiscovery[]
}

export const fetchDayStream = (date: string) => read<DayStream>(`/v1/days/${date}/stream`)

/** Un succès débloqué ce jour-là. */
export interface GameAchievement {
  /** `<appid>:<apiname>`. */
  key: string
  name: string
  /** Heure de Paris du déblocage. */
  time: string
}

/** Un jeu de la journée : joué, ou seulement un succès débloqué. */
export interface DayGame {
  /** L'appid Steam. */
  key: string
  name: string
  /** L'union de ses parties ce jour-là ; 0 pour un jeu qui n'a qu'un succès. */
  minutes: number
  /** Heure de Paris du premier début, qui peut tomber la veille ; `null` sans partie ce jour-là. */
  first_time: string | null
  /** Heure de Paris de la dernière fin, qui peut tomber le lendemain ; `null` sans partie. */
  last_time: string | null
  /** Le total de Steam à la fin de ce jour ; `null` tant qu'aucun relevé ne le donne. */
  total_minutes: number | null
  achievements: GameAchievement[]
}

export interface DayGames {
  date: string
  /** L'union de toutes les parties du jour. */
  minutes: number
  /** Les tranches de jeu qui touchent ce jour-là ; 0 : aucune partie reçue. */
  segments: number
  /** Les succès débloqués ce jour-là. */
  achievements: number
  /** Les jeux joués par temps décroissant puis par nom, puis ceux qui n'ont qu'un succès. */
  games: DayGame[]
  /** Les jeux joués ce jour-là pour la première fois, par première heure ; jamais un jeu déjà joué avant le suivi. */
  discoveries: GameDiscovery[]
}

/** Un jeu joué pour la première fois ce jour-là. */
export interface GameDiscovery {
  level: 'game'
  /** L'appid Steam. */
  key: string
  name: string
}

export const fetchDayGames = (date: string) => read<DayGames>(`/v1/days/${date}/games`)
