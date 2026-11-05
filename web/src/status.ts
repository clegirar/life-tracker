// La page d'état des trackers : ce que dit chaque carte. Le backend ne donne que des codes (état
// rapporté, raison, échéances) et des instants ; tout le texte naît ici, en heure de Paris.

import type { ReportedStatus, SourceStatus } from './api'
import { plural } from './music'
import { addDays, parisDate, parisTime } from './time'

/** L'état d'une carte, en un mot. */
export type CardState = 'ok' | 'todo' | 'broken' | 'silent'

export const STATE_WORD: Record<CardState, string> = {
  ok: 'Fonctionne',
  todo: 'À faire',
  broken: 'En panne',
  silent: 'Silencieux',
}

/** La carte prévient tant de jours avant l'échéance d'une clé. */
export const EXPIRY_WARNING_DAYS = 14
/** Le seuil de fraîcheur des écoutes dans le registre des métriques (`music.listen`, 3 jours). */
const LISTEN_FRESHNESS_MS = 3 * 86_400_000

const DAY_MS = 86_400_000
const NBSP = ' '

// Un remplacement de token crée une nouvelle source : `garmin-2`, `lastfm-2`.
const isGarmin = (name: string) => /^garmin\b/.test(name)
const isLastfm = (name: string) => /^lastfm\b/.test(name)

/** « 9 h 05 », heure de Paris, insécable. */
export function clock(instant: number): string {
  const [hour, minute] = parisTime(instant).split(':')
  return `${Number(hour)}${NBSP}h${NBSP}${minute}`
}

/** « 4 nov. », avec l'année si ce n'est pas celle d'aujourd'hui. */
export function shortDate(date: string, today: string): string {
  const year = date.slice(0, 4) === today.slice(0, 4) ? undefined : 'numeric'
  return new Intl.DateTimeFormat('fr-FR', { timeZone: 'UTC', day: 'numeric', month: 'short', year })
    .format(new Date(`${date}T12:00:00Z`))
    .replace(' ', NBSP)
}

/** « aujourd'hui », « hier », sinon « le 3 nov. ». */
export function dayWord(date: string, today: string): string {
  if (date === today) return "aujourd'hui"
  if (date === addDays(today, -1)) return 'hier'
  return `le ${shortDate(date, today)}`
}

/** Un instant passé : « il y a 10 min », « il y a 2 h », puis le jour et l'heure. */
export function since(instant: number, now: number): string {
  const elapsed = now - instant
  if (elapsed < 60_000) return "à l'instant"
  if (elapsed < 3_600_000) return `il y a ${Math.floor(elapsed / 60_000)}${NBSP}min`
  if (elapsed < 6 * 3_600_000) return `il y a ${Math.floor(elapsed / 3_600_000)}${NBSP}h`
  return `${dayWord(parisDate(instant), parisDate(now))} à ${clock(instant)}`
}

/** Un instant à venir : « à 14 h 05 », « demain à 7 h 00 », « le 6 nov. à 7 h 00 ». */
export function until(instant: number, now: number): string {
  const date = parisDate(instant)
  const today = parisDate(now)
  if (date === today) return `à ${clock(instant)}`
  if (date === addDays(today, 1)) return `demain à ${clock(instant)}`
  return `le ${shortDate(date, today)} à ${clock(instant)}`
}

/** Une ligne de la carte : ce qu'elle dit, et sa valeur. */
export interface Line {
  label: string
  value: string
  /** Une valeur qui demande de l'attention (une échéance proche). */
  warn?: boolean
}

/**
 * La marche à suivre. Les étapes peuvent citer un fichier ou une commande entre accents graves.
 * Un bouton (« Renouveler l'accord ») trouvera sa place ici quand un connecteur saura le proposer.
 */
export interface Guidance {
  title: string
  steps: string[]
  note: string | null
}

export interface Card {
  name: string
  title: string
  state: CardState
  lines: Line[]
  guidance: Guidance | null
  /** Une remarque douce qui ne change pas l'état (des écoutes qui n'arrivent plus). */
  hint: string | null
}

const time = (value: string | null) => (value === null ? null : Date.parse(value))

/**
 * L'état rapporté qui compte encore. Une erreur rapportée avant un contact réussi est passée : la
 * source a eu un contact depuis, sans être muette. L'échéance et la prochaine passe restent valables.
 */
export function currentStatus(source: SourceStatus): ReportedStatus | null {
  const { status } = source
  if (!status) return null
  const seen = time(source.last_seen_at)
  const reported = time(source.status_reported_at)
  if (status.state === 'error' && !source.silent && seen !== null && reported !== null && seen > reported) {
    return { ...status, state: 'ok', reason: null }
  }
  return status
}

/** La date affichée d'une échéance : elle vaut tout le jour qui précède son instant (minuit à Paris). */
const expiryDate = (expires: number) => parisDate(expires - 1)

/** Jours de calendrier, à Paris, jusqu'au dernier jour valable : 0 le jour même, négatif une fois échue. */
function daysLeft(expires: number, now: number): number {
  const day = (date: string) => Date.parse(`${date}T00:00:00Z`)
  return Math.round((day(expiryDate(expires)) - day(parisDate(now))) / DAY_MS)
}

export function expiresSoon(status: ReportedStatus | null, now: number): boolean {
  const expires = time(status?.expires_at ?? null)
  return expires !== null && (expires <= now || daysLeft(expires, now) <= EXPIRY_WARNING_DAYS)
}

export function cardState(source: SourceStatus, now: number): CardState {
  const status = currentStatus(source)
  if (status?.state === 'error') return 'broken'
  if (status?.state === 'action_required' || expiresSoon(status, now)) return 'todo'
  if (source.silent) return 'silent'
  return 'ok'
}

/** Une carte demande une action. Un silence s'affiche, mais se résout souvent seul. */
export const needsAction = (state: CardState) => state === 'todo' || state === 'broken'

const GARMIN_KEY_STEPS = [
  'Dans Garmin Connect, Paramètres, Personal API Key : crée une nouvelle clé et note sa date de fin.',
  'Sur le NAS, dans `secrets/collector.env`, remplace `GARMIN_API_KEY` par la nouvelle clé et `GARMIN_KEY_EXPIRES_ON` par sa date (AAAA-MM-JJ).',
  'Relance le collecteur : `sudo docker compose up -d collector`.',
  "Révoque l'ancienne clé dans Garmin Connect.",
]
const GARMIN_RUNBOOK = 'Le détail : `docs/deploiement.md`, « Mise à jour : Garmin Connect », étape 6.'

/** La variable de la clé, par connecteur. */
const KEY_VARIABLE: [RegExp, string][] = [[/^lastfm\b/, 'LASTFM_API_KEY']]

function keyRejected(name: string): Guidance {
  if (isGarmin(name)) {
    return { title: 'Garmin refuse la clé : elle a expiré ou a été révoquée.', steps: GARMIN_KEY_STEPS, note: GARMIN_RUNBOOK }
  }
  const variable = KEY_VARIABLE.find(([pattern]) => pattern.test(name))?.[1]
  return {
    title: 'Le service refuse la clé du collecteur : elle a expiré ou a été révoquée.',
    steps: [
      'Crée une nouvelle clé chez le service.',
      variable
        ? `Sur le NAS, dans \`secrets/collector.env\`, remplace \`${variable}\` par la nouvelle clé.`
        : 'Sur le NAS, remplace-la dans `secrets/collector.env`.',
      'Relance le collecteur : `sudo docker compose up -d collector`.',
    ],
    note: null,
  }
}

function expiring(name: string, expires: number, now: number): Guidance {
  const date = shortDate(expiryDate(expires), parisDate(now))
  const past = daysLeft(expires, now) < 0 || expires <= now
  const title = past ? `La clé a expiré le ${date} : remplace-la.` : `La clé expire le ${date} : remplace-la avant.`
  if (isGarmin(name)) return { title, steps: GARMIN_KEY_STEPS, note: GARMIN_RUNBOOK }
  return { ...keyRejected(name), title }
}

const LOGS = 'si cela dure, regarde ses logs sur le NAS : `sudo docker compose logs collector`.'

function byReason(source: SourceStatus, reason: ReportedStatus['reason']): Guidance | null {
  switch (reason) {
    case 'key_rejected':
      return keyRejected(source.name)
    case 'consent_expired':
      return {
        title: "L'accord donné au service a expiré : la collecte attend que tu le redonnes.",
        steps: [],
        note: "Un bouton « Renouveler l'accord » viendra ici. En attendant, suis la marche du runbook pour ce service.",
      }
    case 'rate_limited':
      return {
        title: "Le service limite les appels pour l'instant.",
        steps: [],
        note: "Le collecteur réessaie seul à la prochaine passe. Rien à faire, sauf si cela dure plus d'un jour.",
      }
    case 'service_unavailable':
      return {
        title: 'Le service ne répond pas, ou refuse la demande.',
        steps: [],
        note: `Le collecteur réessaie seul ; ${LOGS}`,
      }
    case 'ingest_failed':
      return {
        title: "Le collecteur n'arrive pas à remettre ses données au serveur.",
        steps: [],
        note: 'Il réessaie seul ; si cela dure, vérifie que le backend tourne sur le NAS : `sudo docker compose ps backend`.',
      }
    case null:
      return null
  }
}

function silence(source: SourceStatus): Guidance {
  const hours = Math.round((source.silence_threshold_seconds ?? 0) / 3600)
  const service = isLastfm(source.name) ? 'avec Last.fm ' : ''
  return {
    title: `Aucun contact réussi ${service}depuis plus de ${hours}${NBSP}h : des données peuvent manquer.`,
    steps: [
      'Vérifie que le collecteur tourne sur le NAS : `sudo docker compose ps collector`.',
      'Puis lis ses dernières lignes : `sudo docker compose logs collector`.',
    ],
    note: "Si le service lui-même est en panne, il n'y a qu'à attendre : la collecte reprend seule.",
  }
}

export function guidance(source: SourceStatus, state: CardState, now: number): Guidance | null {
  const status = currentStatus(source)
  if (state === 'broken') return byReason(source, status?.reason ?? null)
  if (state === 'todo') {
    if (status?.state === 'action_required') return byReason(source, status.reason)
    const expires = time(status?.expires_at ?? null)
    return expires === null ? null : expiring(source.name, expires, now)
  }
  if (state === 'silent') return silence(source)
  return null
}

/** Les métriques au jour (une valeur par jour, datée de midi) : le jour compte, pas l'heure. */
const DAILY_METRICS = new Set(['steps', 'calories.burned', 'sleep'])

const LAST_OF: Record<string, string> = {
  coffee: 'Dernier café',
  'music.listen': 'Dernière écoute',
  workout: 'Dernière séance',
}

export function metricLine(metric: SourceStatus['metrics'][number], now: number): Line {
  const at = Date.parse(metric.last_at)
  if (DAILY_METRICS.has(metric.metric)) return { label: metric.label, value: dayWord(parisDate(at), parisDate(now)) }
  return { label: LAST_OF[metric.metric] ?? `Dernière donnée, ${metric.label.toLowerCase()}`, value: since(at, now) }
}

function expiryLine(source: SourceStatus, status: ReportedStatus | null, now: number): Line | null {
  const expires = time(status?.expires_at ?? null)
  if (expires === null) return null
  const label = isGarmin(source.name) ? 'Clé' : 'Échéance'
  const date = shortDate(expiryDate(expires), parisDate(now))
  const left = daysLeft(expires, now)
  if (expires <= now) return { label, value: `expirée le ${date}`, warn: true }
  if (left <= EXPIRY_WARNING_DAYS) {
    const when = left === 0 ? 'ce soir' : left === 1 ? 'demain' : `dans ${plural(left, 'jour')}`
    return { label, value: `expire le ${date}, ${when}`, warn: true }
  }
  return { label, value: `valable jusqu'au ${date}` }
}

export function sourceCard(source: SourceStatus, now: number): Card {
  const state = cardState(source, now)
  const status = currentStatus(source)
  // Sans connecteur (le raccourci café), pas de « passe » : un contact.
  const collected = source.status !== null || source.silence_threshold_seconds !== null
  const seen = time(source.last_seen_at)
  const lines: Line[] = [
    collected
      ? { label: 'Dernière passe', value: seen === null ? "aucune pour l'instant" : since(seen, now) }
      : { label: 'Dernier contact', value: seen === null ? "aucun pour l'instant" : since(seen, now) },
  ]
  const next = time(status?.next_run_at ?? null)
  if (next !== null && next > now) lines.push({ label: 'Prochaine passe', value: until(next, now) })
  const expiry = expiryLine(source, status, now)
  if (expiry) lines.push(expiry)
  lines.push(...source.metrics.map((metric) => metricLine(metric, now)))

  let hint: string | null = null
  const listen = source.metrics.find((metric) => metric.metric === 'music.listen')
  if (state === 'ok' && isLastfm(source.name) && listen && now - Date.parse(listen.last_at) > LISTEN_FRESHNESS_MS) {
    hint = 'Aucune écoute depuis plus de 3 jours. Pause musicale, ou scrobbler arrêté ? Ouvre-le sur le téléphone et vérifie que Last.fm reçoit tes écoutes.'
  }

  return {
    name: source.name,
    title: source.label ?? source.name,
    state,
    lines,
    guidance: guidance(source, state, now),
    hint,
  }
}

/** Le sous-titre de la page. */
export function summary(cards: Card[]): string {
  const waiting = cards.filter((card) => needsAction(card.state)).length
  if (waiting > 0) return waiting === 1 ? 'Un tracker attend quelque chose de toi' : `${waiting} trackers attendent quelque chose de toi`
  const silent = cards.filter((card) => card.state === 'silent').length
  if (silent > 0) return silent === 1 ? 'Un tracker silencieux' : `${silent} trackers silencieux`
  return 'Tout fonctionne'
}

/** Découpe une étape en texte et en code (entre accents graves). */
export function codeParts(text: string): { code: boolean; text: string }[] {
  return text
    .split('`')
    .map((part, index) => ({ code: index % 2 === 1, text: part }))
    .filter((part) => part.text !== '')
}
