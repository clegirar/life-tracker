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
/** Le connecteur YouTube passe en erreur sans export reçu depuis 48 h (`EXPORT_STALE_AFTER`). */
const EXPORT_STALE_MS = 2 * DAY_MS
const NBSP = ' '

// Un remplacement de token crée une nouvelle source : `garmin-2`, `lastfm-2`.
const isGarmin = (name: string) => /^garmin\b/.test(name)
const isLastfm = (name: string) => /^lastfm\b/.test(name)
const isYoutube = (name: string) => /^youtube\b/.test(name)
const isTwitch = (name: string) => /^twitch\b/.test(name)
const isSteam = (name: string) => /^steam\b/.test(name)

/** Comment une source dit son échéance. */
interface ExpiryPolicy {
  label: string
  /** La ligne passe en couleur tant de jours avant. */
  warnDays: number
  /** L'échéance proche suffit à rendre la carte « À faire ». */
  todo: boolean
  /** L'heure compte (un jeton), pas seulement le jour (une clé). */
  precise: boolean
}

const KEY_POLICY: ExpiryPolicy = { label: 'Échéance', warnDays: EXPIRY_WARNING_DAYS, todo: true, precise: false }

/**
 * Le jeton de Google dure 7 jours : prévenir 14 jours avant mettrait la carte toujours « À faire ». Le
 * connecteur redemande l'accord lui-même la veille, et c'est ce qui rend la carte « À faire ».
 */
function expiryPolicy(name: string): ExpiryPolicy {
  if (isGarmin(name)) return { ...KEY_POLICY, label: 'Clé' }
  if (isYoutube(name)) return { label: 'Accès Google', warnDays: 2, todo: false, precise: true }
  return KEY_POLICY
}

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

/** Une échéance proche (orange), ou passée (rouge). */
export type Urgency = 'soon' | 'past'

/** Une ligne de la carte : ce qu'elle dit, et sa valeur. */
export interface Line {
  label: string
  value: string
  /** Une valeur qui demande de l'attention : une échéance proche, ou passée. */
  warn?: Urgency
}

/** Le bouton de la marche à suivre : une page à ouvrir dans l'onglet même (Google revient sur l'app). */
export interface Action {
  label: string
  href: string
}

/** La marche à suivre. Les étapes peuvent citer un fichier ou une commande entre accents graves. */
export interface Guidance {
  title: string
  steps: string[]
  note: string | null
  action?: Action
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

export function expiresSoon(status: ReportedStatus | null, now: number, warnDays = EXPIRY_WARNING_DAYS): boolean {
  const expires = time(status?.expires_at ?? null)
  return expires !== null && (expires <= now || daysLeft(expires, now) <= warnDays)
}

export function cardState(source: SourceStatus, now: number): CardState {
  const status = currentStatus(source)
  const policy = expiryPolicy(source.name)
  if (status?.state === 'error') return 'broken'
  // Une page de consentement attend : le connecteur attend que je l'ouvre.
  if (status?.state === 'action_required' || source.authorization_url) return 'todo'
  if (policy.todo && expiresSoon(status, now, policy.warnDays)) return 'todo'
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
const KEY_VARIABLE: [RegExp, string][] = [
  [/^lastfm\b/, 'LASTFM_API_KEY'],
  [/^steam\b/, 'STEAM_API_KEY'],
]

function keyRejected(name: string): Guidance {
  if (isGarmin(name)) {
    return { title: 'Garmin refuse la clé : elle a expiré ou a été révoquée.', steps: GARMIN_KEY_STEPS, note: GARMIN_RUNBOOK }
  }
  const variable = KEY_VARIABLE.find(([pattern]) => pattern.test(name))?.[1]
  if (isSteam(name)) {
    return {
      title: 'Steam refuse la clé du collecteur : elle a été révoquée, ou le compte a changé.',
      steps: [
        "Sur steamcommunity.com/dev/apikey, révoque l'ancienne clé puis crée-en une nouvelle : Steam n'en donne qu'une par compte.",
        `Sur le NAS, dans \`secrets/collector.env\`, remplace \`${variable}\` par la nouvelle clé.`,
        'Relance le collecteur : `sudo docker compose up -d collector`.',
      ],
      note: "Rien n'est perdu : le total de chaque jeu reprend au relevé suivant, et le temps d'entre-temps se compte à la fin.",
    }
  }
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

/**
 * Un accord à donner ou à redonner à Google. Le bouton suit `authorization_url`, quelle que soit la
 * source ; sans elle, le connecteur ne l'a pas encore préparée.
 */
function consent(source: SourceStatus, reason: 'consent_required' | 'consent_expired'): Guidance {
  const first = reason === 'consent_required'
  const title = first
    ? 'La collecte attend ton accord pour commencer.'
    : "L'accord donné à Google est à renouveler : la collecte attend que tu le redonnes."
  if (!source.authorization_url) {
    return {
      title,
      steps: [],
      note: "Le lien vers Google arrive d'ici quelques minutes, à la prochaine passe du collecteur. Si rien ne vient, regarde ses logs sur le NAS : `sudo docker compose logs collector`.",
    }
  }
  return {
    title,
    steps: [
      "Google s'ouvre dans cette page : choisis ton compte et accepte l'accès demandé.",
      "Si Google prévient que l'application n'est pas validée, continue : c'est la tienne.",
      'Google te ramène ici, et la collecte reprend seule dans la minute.',
    ],
    note: first ? null : "Rien n'est perdu : l'export suivant reprend depuis la dernière donnée reçue.",
    action: { label: first ? "Donner l'accord" : "Renouveler l'accord", href: source.authorization_url },
  }
}

const LOGS = 'si cela dure, regarde ses logs sur le NAS : `sudo docker compose logs collector`.'

/** YouTube : Google répond, mais aucun export n'a abouti depuis deux jours. */
function exportsStale(source: SourceStatus, status: ReportedStatus | null, now: number): boolean {
  if (!isYoutube(source.name) || status?.reason !== 'service_unavailable') return false
  const collected = time(status.last_collected_at ?? null)
  return collected === null || now - collected > EXPORT_STALE_MS
}

/** Les deux réglages de l'extension Twitch, sur le Mac : son module et sa page d'options. */
const TWITCH_ADDON = "Dans Firefox, sur le Mac, ouvre `about:addons` : le module « Life tracker : Twitch » doit être activé."
const TWITCH_OPTIONS = "Ouvre sa page d'options (le bouton de l'extension, ou Préférences dans `about:addons`) : elle montre la dernière erreur ; « Tester » envoie un signal."
const TWITCH_TOKEN =
  "Un « ! » sur le bouton : le token est refusé. Recrée la source `twitch` sur le NAS (`docs/deploiement.md`, « Mise à jour : Twitch »), colle le nouveau token dans la page d'options, puis « Tester »."

function byReason(source: SourceStatus, reason: ReportedStatus['reason'], now: number): Guidance | null {
  if (isTwitch(source.name) && reason === 'ingest_failed') {
    return {
      title: "L'extension n'arrive pas à remettre ses segments au serveur.",
      steps: [TWITCH_OPTIONS, 'Si cela dure, vérifie que le backend tourne sur le NAS : `sudo docker compose ps backend`.'],
      note: "Rien n'est perdu : l'extension garde les segments dans Firefox et les renvoie seule.",
    }
  }
  if (exportsStale(source, currentStatus(source), now)) {
    return {
      title: "Google répond, mais aucun export n'a abouti depuis 2 jours.",
      steps: [],
      note: `Le collecteur réessaie seul à chaque export permis ; ${LOGS}`,
    }
  }
  switch (reason) {
    case 'key_rejected':
      return keyRejected(source.name)
    case 'consent_required':
    case 'consent_expired':
      return consent(source, reason)
    case 'profile_private':
      return {
        title: 'Le profil Steam est privé : Steam ne donne ni les jeux ni les succès.',
        steps: [
          'Sur steamcommunity.com, ouvre ton profil, puis Modifier le profil, Paramètres de confidentialité.',
          'Passe « Mon profil » et « Détails des jeux » en Public.',
          "La collecte reprend seule à la prochaine passe, dans un quart d'heure au plus.",
        ],
        note: "Rien n'est perdu : le temps joué pendant ce temps se compte à la reprise, et les succès gardent leur date.",
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

/** « 6 h », « 3 jours » : un seuil de 48 h ou plus se dit en jours. */
export function thresholdText(seconds: number): string {
  const hours = Math.round(seconds / 3600)
  return hours >= 48 && hours % 24 === 0 ? plural(hours / 24, 'jour').replace(' ', NBSP) : `${hours}${NBSP}h`
}

function silence(source: SourceStatus): Guidance {
  const threshold = thresholdText(source.silence_threshold_seconds ?? 0)
  if (isTwitch(source.name)) {
    return {
      title: `Aucun signal de Firefox depuis plus de ${threshold} : des streams peuvent manquer.`,
      steps: ["Ouvre Firefox sur le Mac : l'extension envoie un signal toutes les 15 minutes tant qu'il est ouvert.", TWITCH_ADDON, TWITCH_OPTIONS, TWITCH_TOKEN],
      note: "Firefox simplement fermé ? Rien n'est perdu : ce qu'il n'a pas pu envoyer part à sa réouverture.",
    }
  }
  const service = isLastfm(source.name) ? 'avec Last.fm ' : isSteam(source.name) ? 'avec Steam ' : ''
  return {
    title: `Aucun contact réussi ${service}depuis plus de ${threshold} : des données peuvent manquer.`,
    steps: [
      'Vérifie que le collecteur tourne sur le NAS : `sudo docker compose ps collector`.',
      'Puis lis ses dernières lignes : `sudo docker compose logs collector`.',
    ],
    note: "Si le service lui-même est en panne, il n'y a qu'à attendre : la collecte reprend seule.",
  }
}

export function guidance(source: SourceStatus, state: CardState, now: number): Guidance | null {
  const status = currentStatus(source)
  if (state === 'broken') return byReason(source, status?.reason ?? null, now)
  if (state === 'todo') {
    if (status?.state === 'action_required') return byReason(source, status.reason, now)
    if (source.authorization_url) return consent(source, 'consent_expired')
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
  'video.watch': 'Dernière vidéo',
  'stream.watch': 'Dernier visionnage',
  'game.play': 'Dernière partie',
  'game.achievement': 'Dernier succès',
}

export function metricLine(metric: SourceStatus['metrics'][number], now: number): Line {
  const at = Date.parse(metric.last_at)
  if (DAILY_METRICS.has(metric.metric)) return { label: metric.label, value: dayWord(parisDate(at), parisDate(now)) }
  return { label: LAST_OF[metric.metric] ?? `Dernière donnée, ${metric.label.toLowerCase()}`, value: since(at, now) }
}

/**
 * L'échéance que rapporte une source (la clé Garmin, l'accès Google), dite comme sur la carte d'état :
 * proche dans la fenêtre de sa politique (14 jours pour une clé, 2 jours pour le jeton Google de 7 jours).
 * La page Sources la reprend telle quelle, pour que les deux pages disent la même chose.
 */
export function expiryLine(source: SourceStatus, now: number): Line | null {
  const expires = time(currentStatus(source)?.expires_at ?? null)
  if (expires === null) return null
  const { label, warnDays, precise } = expiryPolicy(source.name)
  if (precise) {
    // Un jeton échoit à une heure précise : « expire demain à 14 h 32 ».
    if (expires <= now) return { label, value: `expiré ${dayWord(parisDate(expires), parisDate(now))} à ${clock(expires)}`, warn: 'past' }
    if (expires - now <= warnDays * DAY_MS) return { label, value: `expire ${until(expires, now)}`, warn: 'soon' }
    return { label, value: `valable jusqu'au ${shortDate(parisDate(expires), parisDate(now))} à ${clock(expires)}` }
  }
  const date = shortDate(expiryDate(expires), parisDate(now))
  const left = daysLeft(expires, now)
  if (expires <= now) return { label, value: `expirée le ${date}`, warn: 'past' }
  if (left <= warnDays) {
    const when = left === 0 ? 'ce soir' : left === 1 ? 'demain' : `dans ${plural(left, 'jour')}`
    return { label, value: `expire le ${date}, ${when}`, warn: 'soon' }
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
    // L'extension Twitch n'a pas de passe : un battement toutes les 15 min, tant que Firefox est ouvert.
    isTwitch(source.name)
      ? { label: 'Dernier signal de Firefox', value: seen === null ? "aucun pour l'instant" : since(seen, now) }
      : collected
        ? { label: 'Dernière passe', value: seen === null ? "aucune pour l'instant" : since(seen, now) }
        : { label: 'Dernier contact', value: seen === null ? "aucun pour l'instant" : since(seen, now) },
  ]
  // La dernière collecte réussie : pour YouTube, le dernier export, attendu même avant le premier.
  const lastCollected = time(status?.last_collected_at ?? null)
  if (isYoutube(source.name) && status) {
    lines.push({ label: 'Dernier export', value: lastCollected === null ? "aucun pour l'instant" : since(lastCollected, now) })
  } else if (lastCollected !== null) {
    // Twitch : le dernier lot de segments reçu par le serveur.
    lines.push({ label: isTwitch(source.name) ? 'Dernier envoi' : 'Dernière collecte', value: since(lastCollected, now) })
  }
  const next = time(status?.next_run_at ?? null)
  // YouTube : la prochaine passe est le prochain export que Google permet.
  if (next !== null && next > now) lines.push({ label: isYoutube(source.name) ? 'Prochain export' : 'Prochaine passe', value: until(next, now) })
  const expiry = expiryLine(source, now)
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

/** Découpe une étape en texte et en code (entre accents graves). */
export function codeParts(text: string): { code: boolean; text: string }[] {
  return text
    .split('`')
    .map((part, index) => ({ code: index % 2 === 1, text: part }))
    .filter((part) => part.text !== '')
}
