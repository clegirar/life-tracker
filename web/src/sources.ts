// La page Sources : une fiche par tracker (comment ses données arrivent, ses clés) et les autres clés à
// garder en vie. Tout le texte est ici, figé ; seules les échéances que le serveur connaît (la clé Garmin,
// l'accès Google) et l'état de chaque source viennent de `GET /v1/sources/status`, comme pour la page État.
//
// Aucune valeur secrète, ni nom d'hôte du NAS ou du tailnet : seulement le nom de chaque clé et son échéance.

import type { SourceStatus, SourcesStatus } from './api'
import { cardState, currentStatus, expiryLine, type CardState, type Urgency } from './status'

export interface KeyInfo {
  /** Le nom de la clé, et sa variable entre accents graves s'il y en a une. */
  name: string
  /** L'échéance connue d'avance, ou ce qui s'affiche quand le serveur n'en rapporte pas. */
  expiry: string
  /** L'échéance est celle que rapporte la source de la fiche (`expires_at`). */
  live?: boolean
}

export interface TrackerSheet {
  id: string
  title: string
  /** Le nom des sources qui l'alimentent : un remplacement de token crée `garmin-2`, `lastfm-2`. */
  source: RegExp
  /** Comment les données arrivent : qui collecte, à quel rythme, par où. */
  arrival: string
  keys: KeyInfo[]
}

export interface KeyGroup {
  id: string
  title: string
  keys: KeyInfo[]
}

const NO_EXPIRY = 'Sans échéance.'
const NO_KNOWN_EXPIRY = 'Sans échéance connue.'

/** Le token d'une source : il n'expire jamais, et ne se réutilise pas une fois révoqué. */
function sourceToken(name: string): KeyInfo {
  return { name: `Token de la source \`${name}\``, expiry: `${NO_EXPIRY} Le serveur n'en garde que le haché.` }
}

export const TRACKERS: TrackerSheet[] = [
  {
    id: 'cafe',
    title: 'Café',
    source: /^ios\b/,
    arrival:
      "Le raccourci iOS envoie chaque café à l'API d'ingestion, par Tailscale, au moment où je le lance. La web app en note aussi, avec ma session, sans clé à part.",
    keys: [sourceToken('ios')],
  },
  {
    id: 'musique',
    title: 'Musique',
    source: /^lastfm\b/,
    arrival:
      "Le collecteur du NAS interroge Last.fm toutes les 5 min et relit les 48 dernières heures. L'historique d'avant vient de l'export Apple Music, importé une fois à la main (`import-apple`).",
    keys: [
      { name: "Clé d'API Last.fm (`LASTFM_API_KEY`)", expiry: NO_KNOWN_EXPIRY },
      sourceToken('lastfm'),
      sourceToken('apple'),
    ],
  },
  {
    id: 'garmin',
    title: 'Garmin Connect',
    source: /^garmin\b/,
    arrival:
      "Le collecteur du NAS interroge l'API personnelle de Garmin Connect tous les quarts d'heure et relit les 7 derniers jours : pas, calories, sommeil, séances et leurs traces.",
    keys: [
      {
        name: "Clé d'API personnelle Garmin (`GARMIN_API_KEY`)",
        expiry: "1 an au plus ; date inconnue du serveur tant que `GARMIN_KEY_EXPIRES_ON` manque. Garmin prévient par courriel une semaine avant.",
        live: true,
      },
      sourceToken('garmin'),
    ],
  },
  {
    id: 'youtube',
    title: 'YouTube',
    source: /^youtube\b/,
    arrival:
      "Le collecteur du NAS passe toutes les 5 min et demande un export Google Data Portability au plus une fois par 24 h, puis complète chaque vidéo par l'API YouTube Data v3.",
    keys: [
      {
        name: 'Accord Google',
        expiry: '7 jours (écran de consentement en mode Testing) ; aucun accord en cours.',
        live: true,
      },
      { name: 'Client OAuth Google (`YOUTUBE_CLIENT_ID`, `YOUTUBE_CLIENT_SECRET`)', expiry: NO_KNOWN_EXPIRY },
      { name: "Clé d'API YouTube Data v3 (`YOUTUBE_API_KEY`)", expiry: NO_KNOWN_EXPIRY },
      sourceToken('youtube'),
    ],
  },
  {
    id: 'twitch',
    title: 'Twitch',
    source: /^twitch\b/,
    arrival:
      "L'extension Firefox du Mac mesure la lecture en segments de 60 s au plus et les envoie à l'API d'ingestion chaque minute. Elle bat toutes les 15 min tant que Firefox est ouvert.",
    keys: [sourceToken('twitch')],
  },
  {
    id: 'steam',
    title: 'Steam',
    source: /^steam\b/,
    arrival:
      "Le collecteur du NAS interroge l'API Web de Steam tous les quarts d'heure pour le compte de `STEAM_ID` : le total de chaque jeu, dont l'écart entre deux relevés donne le temps joué, et les succès avec leur date. Le profil et ses « Détails des jeux » doivent être publics.",
    keys: [{ name: "Clé d'API Web Steam (`STEAM_API_KEY`)", expiry: NO_EXPIRY }, sourceToken('steam')],
  },
]

export const OTHER_KEYS: KeyGroup[] = [
  {
    id: 'claude',
    title: 'Claude (serveur MCP)',
    keys: [
      { name: 'Jeton de service `mcp` (`MCP_SERVICE_TOKEN`)', expiry: NO_EXPIRY },
      { name: 'Secret de connexion du MCP (`MCP_LOGIN_HASH`)', expiry: NO_EXPIRY },
      {
        name: 'Autorisation de Claude (jetons OAuth)',
        expiry: "Jeton d'accès d'une heure, renouvelé seul par Claude ; 30 jours sans usage, se reconnecter.",
      },
      { name: 'Zones de confidentialité (`PRIVACY_ZONES`)', expiry: NO_EXPIRY },
    ],
  },
  {
    id: 'web',
    title: 'Web app',
    keys: [{ name: 'Mot de passe du compte web', expiry: NO_EXPIRY }],
  },
  {
    id: 'base',
    title: 'Base de données',
    keys: [
      { name: 'Mot de passe de `postgres` (`POSTGRES_PASSWORD`)', expiry: NO_EXPIRY },
      { name: 'Mot de passe de `tracker_owner`', expiry: NO_EXPIRY },
      { name: 'Mot de passe de `tracker_app`', expiry: NO_EXPIRY },
      { name: 'Mot de passe de `tracker_admin`', expiry: NO_EXPIRY },
    ],
  },
  {
    id: 'images',
    title: 'Images (ghcr.io)',
    keys: [
      { name: 'Token GitHub `read:packages` du NAS', expiry: 'La date choisie à sa création (token classique), à noter dans le gestionnaire.' },
    ],
  },
  {
    id: 'deploiement',
    title: 'Déploiement automatique (secrets GitHub)',
    keys: [
      { name: 'Clé SSH de déploiement (`NAS_SSH_KEY`)', expiry: NO_EXPIRY },
      { name: 'Client OAuth Tailscale (`TS_OAUTH_CLIENT_ID`, `TS_OAUTH_SECRET`)', expiry: NO_KNOWN_EXPIRY },
      { name: "Clé d'hôte du NAS (`NAS_KNOWN_HOSTS`)", expiry: "À refaire si le NAS change de clé d'hôte (réinstallation)." },
      { name: 'Adresse du NAS (`NAS_HOST`, `NAS_USER`, `NAS_SSH_PORT`)', expiry: NO_EXPIRY },
    ],
  },
  {
    id: 'extension',
    title: 'Extension Firefox (AMO)',
    keys: [{ name: "Clés d'API AMO (`WEB_EXT_API_KEY`, `WEB_EXT_API_SECRET`)", expiry: NO_KNOWN_EXPIRY }],
  },
]

/** La source active d'une fiche, s'il y en a une. */
export function sourceOf(sheet: TrackerSheet, data: SourcesStatus | null): SourceStatus | null {
  return data?.sources.find((source) => sheet.source.test(source.name)) ?? null
}

export interface ShownExpiry {
  text: string
  /** `null` : rien à signaler. */
  urgency: Urgency | null
  /** L'échéance vient du serveur, et non du texte figé. */
  live: boolean
}

/**
 * L'échéance affichée d'une clé. Celle que rapporte la source, dite comme sur la page État et avec ses
 * seuils ; un accord Google à donner ou à redonner passe avant. Sinon, le texte figé.
 */
export function keyExpiry(key: KeyInfo, source: SourceStatus | null, now: number): ShownExpiry {
  const fallback: ShownExpiry = { text: key.expiry, urgency: null, live: false }
  if (!key.live || source === null) return fallback
  const reason = currentStatus(source)?.reason
  if (reason === 'consent_required') return { text: "Accord à donner, depuis la page État.", urgency: 'soon', live: true }
  if (reason === 'consent_expired') return { text: "Accord expiré : à redonner depuis la page État.", urgency: 'past', live: true }
  const line = expiryLine(source, now)
  if (line === null) return fallback
  // « valable jusqu'au 4 nov. 2027 » : la phrase de la carte, avec une majuscule.
  const text = line.value.charAt(0).toUpperCase() + line.value.slice(1)
  return { text: text.endsWith('.') ? text : `${text}.`, urgency: line.warn ?? null, live: true }
}

/** L'état d'une fiche : celui de la carte d'état de sa source, ou `null` sans source active. */
export function sheetState(source: SourceStatus | null, now: number): CardState | null {
  return source === null ? null : cardState(source, now)
}

/**
 * Les espaces insécables du français : avant « ; », « : », « ? », « ! » et « », après « « ». Une ligne ne
 * commence jamais par un point-virgule ni ne se termine par un guillemet ouvrant.
 */
export function frenchSpaces(text: string): string {
  return text.replace(/ ([;:?!»])/g, '\u00a0$1').replace(/« /g, '«\u00a0')
}

/** Toutes les clés de la page, fiches et autres clés. */
export function allKeys(): KeyInfo[] {
  return [...TRACKERS.flatMap((sheet) => sheet.keys), ...OTHER_KEYS.flatMap((group) => group.keys)]
}
