// La page Sources : une fiche par tracker (comment ses données arrivent, ses clés) et les autres clés à
// garder en vie. Tout le texte est ici, figé ; seules les échéances que le serveur connaît (la clé Garmin,
// l'accès Google) et l'état de chaque source viennent de `GET /v1/sources/status`, comme pour la page État.
//
// Aucune valeur secrète, ni nom d'hôte du NAS ou du tailnet : seulement le nom de chaque clé, où elle vit,
// son échéance et la section de docs/deploiement.md qui dit comment la remplacer.

import type { SourceStatus, SourcesStatus } from './api'
import { cardState, currentStatus, expiryLine, type CardState, type Urgency } from './status'

/** Une section de docs/deploiement.md : son titre exact, et au besoin l'étape. */
export interface DocRef {
  section: string
  detail?: string
}

export interface KeyInfo {
  /** Le nom de la clé, et sa variable entre accents graves s'il y en a une. */
  name: string
  role: string
  /** Où elle vit : un fichier `secrets/…` du NAS, un secret GitHub, l'extension, le gestionnaire… */
  where: string
  /** L'échéance connue d'avance, ou ce qui s'affiche quand le serveur n'en rapporte pas. */
  expiry: string
  /** L'échéance est celle que rapporte la source de la fiche (`expires_at`). */
  live?: boolean
  /** Comment la remplacer, en une ou deux phrases. */
  replace: string
  doc: DocRef
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

const COLLECTOR_ENV = '`secrets/collector.env` sur le NAS, et le gestionnaire de mots de passe.'
const RESTART_COLLECTOR = 'puis `sudo docker compose up -d collector`.'
const NO_EXPIRY = 'Sans échéance.'
const NO_KNOWN_EXPIRY = 'Sans échéance connue.'

/** Le token d'une source : il n'expire jamais, et ne se réutilise pas une fois révoqué. */
function sourceToken(name: string, role: string, where: string, replace: string, doc: DocRef): KeyInfo {
  return { name: `Token de la source \`${name}\``, role, where, expiry: `${NO_EXPIRY} Le serveur n'en garde que le haché.`, replace, doc }
}

export const TRACKERS: TrackerSheet[] = [
  {
    id: 'cafe',
    title: 'Café',
    source: /^ios\b/,
    arrival:
      "Le raccourci iOS envoie chaque café à l'API d'ingestion, par Tailscale, au moment où je le lance. La web app en note aussi, avec ma session, sans clé à part.",
    keys: [
      sourceToken(
        'ios',
        "Permet au raccourci d'écrire des cafés, et rien d'autre.",
        'Dans le raccourci iOS (en-tête `Authorization`), et le gestionnaire de mots de passe. Ne jamais partager le raccourci.',
        '`admin revoke-source ios`, puis une source à un nouveau nom (`ios-2`) et son token dans le raccourci.',
        { section: '9. Source iOS' },
      ),
    ],
  },
  {
    id: 'musique',
    title: 'Musique',
    source: /^lastfm\b/,
    arrival:
      "Le collecteur du NAS interroge Last.fm toutes les 5 min et relit les 48 dernières heures. L'historique d'avant vient de l'export Apple Music, importé une fois à la main (`import-apple`).",
    keys: [
      {
        name: "Clé d'API Last.fm (`LASTFM_API_KEY`)",
        role: "Lit mon historique d'écoutes, avec `LASTFM_USER` (un nom, pas un secret).",
        where: COLLECTOR_ENV,
        expiry: NO_KNOWN_EXPIRY,
        replace: `Une nouvelle clé sur last.fm/api/account/create, à la place de \`LASTFM_API_KEY\`, ${RESTART_COLLECTOR}`,
        doc: { section: 'Exploitation', detail: 'Collecteur' },
      },
      sourceToken(
        'lastfm',
        "Permet au collecteur d'écrire les écoutes Last.fm (`COLLECTOR_TOKEN`).",
        COLLECTOR_ENV,
        "`admin revoke-source lastfm`, puis `create-source lastfm-2` et ses deux droits, et le nouveau `COLLECTOR_TOKEN`.",
        { section: "Mise à jour vers l'étape 5 (musique)", detail: 'étape 3' },
      ),
      sourceToken(
        'apple',
        "Sert à l'import de l'export Apple Music. Un futur export doit repasser par cette source pour retomber sur les mêmes clés.",
        '`secrets/apple-import.token` sur le NAS, fichier en 600.',
        'Ni la révoquer, ni perdre le fichier. Son seul droit est retiré après chaque import : `grant-kind apple apple.play_history_row` le redonne.',
        { section: "Mise à jour vers l'étape 7 (historique Apple)", detail: 'étape 3' },
      ),
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
        role: 'Lit mon compte Garmin Connect. Une seule clé par compte : elle ne sert jamais en développement.',
        where: `${COLLECTOR_ENV} Sa date de fin dans \`GARMIN_KEY_EXPIRES_ON\`.`,
        expiry: "1 an au plus ; date inconnue du serveur tant que `GARMIN_KEY_EXPIRES_ON` manque. Garmin prévient par courriel une semaine avant.",
        live: true,
        replace:
          "Chaque année : une nouvelle clé dans Garmin Connect (Paramètres, Personal API Key), à la place de `GARMIN_API_KEY`, avec sa date dans `GARMIN_KEY_EXPIRES_ON`, puis relancer le collecteur et révoquer l'ancienne.",
        doc: { section: 'Mise à jour : Garmin Connect (après le POC)', detail: 'étape 6' },
      },
      sourceToken(
        'garmin',
        "Permet au collecteur d'écrire les données Garmin et leurs révisions (`GARMIN_TOKEN`).",
        COLLECTOR_ENV,
        '`create-source garmin-2` et ses cinq droits, le nouveau `GARMIN_TOKEN` et `GARMIN_SOURCE=garmin-2`.',
        { section: 'Mise à jour : Garmin Connect (après le POC)', detail: 'étape 3' },
      ),
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
        role: "Le jeton qui permet au collecteur de demander l'export de mon historique YouTube.",
        where: "L'état du collecteur (`youtube-state.json`, volume `life-tracker-prod-collector-state`), en 600.",
        expiry: '7 jours (écran de consentement en mode Testing) ; aucun accord en cours.',
        live: true,
        replace: "Un jour avant l'échéance, la carte YouTube de la page État propose « Renouveler l'accord » : le donner depuis l'iPhone ou le Mac.",
        doc: { section: 'Mise à jour : YouTube', detail: 'étapes 4 et 5' },
      },
      {
        name: 'Client OAuth Google (`YOUTUBE_CLIENT_ID`, `YOUTUBE_CLIENT_SECRET`)',
        role: 'Identifie mon application auprès de Google (projet Google Cloud, client « Application Web »).',
        where: COLLECTOR_ENV,
        expiry: NO_KNOWN_EXPIRY,
        replace: `Un nouveau secret du client dans Google Cloud, à la place de \`YOUTUBE_CLIENT_SECRET\`, ${RESTART_COLLECTOR}`,
        doc: { section: 'Mise à jour : YouTube', detail: 'étapes 2 et 3' },
      },
      {
        name: "Clé d'API YouTube Data v3 (`YOUTUBE_API_KEY`)",
        role: 'Lit la durée et la nature de chaque vidéo (classique ou Short). Restreinte à YouTube Data API v3.',
        where: COLLECTOR_ENV,
        expiry: NO_KNOWN_EXPIRY,
        replace: `Une nouvelle clé restreinte dans Google Cloud, à la place de \`YOUTUBE_API_KEY\`, ${RESTART_COLLECTOR}`,
        doc: { section: 'Mise à jour : YouTube', detail: 'étapes 2 et 3' },
      },
      sourceToken(
        'youtube',
        "Permet au collecteur d'écrire les vidéos vues et leurs fiches (`YOUTUBE_TOKEN`).",
        COLLECTOR_ENV,
        '`create-source youtube-2` et ses trois droits, le nouveau `YOUTUBE_TOKEN` et `YOUTUBE_SOURCE=youtube-2`.',
        { section: 'Mise à jour : YouTube', detail: 'étape 3' },
      ),
    ],
  },
  {
    id: 'twitch',
    title: 'Twitch',
    source: /^twitch\b/,
    arrival:
      "L'extension Firefox du Mac mesure la lecture en segments de 60 s au plus et les envoie à l'API d'ingestion chaque minute. Elle bat toutes les 15 min tant que Firefox est ouvert.",
    keys: [
      sourceToken(
        'twitch',
        "Permet à l'extension d'écrire des visionnages (`twitch.watch`), rien d'autre.",
        "La page d'options de l'extension Firefox sur le Mac, et le gestionnaire de mots de passe.",
        "`admin revoke-source twitch`, puis une source à un nouveau nom (`twitch-2`) et son token dans la page d'options, puis « Tester ».",
        { section: 'Mise à jour : Twitch', detail: 'étapes 1 et 2' },
      ),
    ],
  },
]

const DB_PASSWORD_REPLACE =
  "Il ne sert qu'à la création du volume : le changer demande aussi un `ALTER ROLE`, depuis `sudo docker compose exec db psql -U postgres`."
const DB_DOC: DocRef = { section: '5. Configuration et secrets' }

export const OTHER_KEYS: KeyGroup[] = [
  {
    id: 'claude',
    title: 'Claude (serveur MCP)',
    keys: [
      {
        name: 'Jeton de service `mcp` (`MCP_SERVICE_TOKEN`)',
        role: "Le MCP lit l'API de lecture avec lui : lecture seule, pseudonymes, refusé sur toute écriture.",
        where: "`secrets/mcp.env` sur le NAS ; le serveur n'en garde que le haché.",
        expiry: NO_EXPIRY,
        replace: '`admin revoke-service-token mcp`, puis `create-service-token mcp-2` dans `secrets/mcp.env`, puis `sudo docker compose up -d mcp`.',
        doc: { section: "Mise à jour vers l'étape 9 (serveur MCP)", detail: 'étape 3 et « Couper »' },
      },
      {
        name: 'Secret de connexion du MCP (`MCP_LOGIN_HASH`)',
        role: 'Ce que je saisis pour autoriser Claude. 16 caractères au moins, différent du mot de passe web.',
        where: "Le gestionnaire de mots de passe ; `secrets/mcp.env` n'en garde que le haché Argon2id.",
        expiry: NO_EXPIRY,
        replace: "Un nouveau secret, haché par `hash-password` de l'image `mcp`, à la place de `MCP_LOGIN_HASH`, puis `sudo docker compose up -d mcp`.",
        doc: { section: "Mise à jour vers l'étape 9 (serveur MCP)", detail: 'étape 4' },
      },
      {
        name: 'Autorisation de Claude (jetons OAuth)',
        role: 'Les jetons que le MCP remet au connecteur de Claude après ma connexion.',
        where: 'Le volume `life-tracker-prod-mcp-state` sur le NAS, hachés.',
        expiry: "Jeton d'accès d'une heure, renouvelé seul par Claude ; 30 jours sans usage, se reconnecter.",
        replace: 'Pour couper : retirer le connecteur dans Claude et effacer le volume. Pour se reconnecter : « Se connecter » dans les réglages du connecteur.',
        doc: { section: "Mise à jour vers l'étape 9 (serveur MCP)", detail: 'étape 8 et « Couper »' },
      },
      {
        name: 'Zones de confidentialité (`PRIVACY_ZONES`)',
        role: "Pas une clé, mais aussi secrète : elles cachent à Claude les coordonnées et les parcours autour de chez moi.",
        where: '`secrets/backend.env` sur le NAS, et nulle part ailleurs : ni dépôt, ni conversation, ni logs.',
        expiry: NO_EXPIRY,
        replace: 'La ligne `PRIVACY_ZONES` de `secrets/backend.env`, puis `sudo docker compose up -d backend`.',
        doc: { section: 'Mise à jour : Garmin Connect (après le POC)', detail: '« Zone de confidentialité du MCP »' },
      },
    ],
  },
  {
    id: 'web',
    title: 'Web app',
    keys: [
      {
        name: 'Mot de passe du compte web',
        role: 'Ma connexion à la web app, le seul compte. Une session dure 30 jours.',
        where: "Le gestionnaire de mots de passe ; la base n'en garde que le haché.",
        expiry: NO_EXPIRY,
        replace: '`sudo docker compose run --rm admin set-password moi`, qui ferme aussi les sessions ouvertes.',
        doc: { section: '8. Compte web' },
      },
    ],
  },
  {
    id: 'base',
    title: 'Base de données',
    keys: [
      {
        name: 'Mot de passe de `postgres` (`POSTGRES_PASSWORD`)',
        role: "Le superutilisateur de PostgreSQL. Aucun service ne s'en sert.",
        where: '`secrets/db.env` sur le NAS.',
        expiry: NO_EXPIRY,
        replace: DB_PASSWORD_REPLACE,
        doc: DB_DOC,
      },
      {
        name: 'Mot de passe de `tracker_owner`',
        role: 'Le propriétaire du schéma, pour les seules migrations ; le backend ne le connaît jamais.',
        where: '`secrets/db.env` et `secrets/migrate.env` (`MIGRATE_DATABASE_URL`) sur le NAS.',
        expiry: NO_EXPIRY,
        replace: DB_PASSWORD_REPLACE,
        doc: DB_DOC,
      },
      {
        name: 'Mot de passe de `tracker_app`',
        role: "Le rôle du backend : l'API et les interprètes.",
        where: '`secrets/db.env` et `secrets/backend.env` (`DATABASE_URL`) sur le NAS.',
        expiry: NO_EXPIRY,
        replace: DB_PASSWORD_REPLACE,
        doc: DB_DOC,
      },
      {
        name: 'Mot de passe de `tracker_admin`',
        role: "L'outil d'administration (`admin`) : sources, droits, reconstruction.",
        where: '`secrets/db.env` et `secrets/admin.env` (`ADMIN_DATABASE_URL`) sur le NAS.',
        expiry: NO_EXPIRY,
        replace: DB_PASSWORD_REPLACE,
        doc: DB_DOC,
      },
    ],
  },
  {
    id: 'images',
    title: 'Images (ghcr.io)',
    keys: [
      {
        name: 'Token GitHub `read:packages` du NAS',
        role: "Permet au NAS de tirer les images privées de ghcr.io. Expiré, le `pull` échoue et le déploiement revient à l'ancien tag.",
        where: 'Le gestionnaire de mots de passe, et la connexion `docker login ghcr.io` de root sur le NAS.',
        expiry: 'La date choisie à sa création (token classique), à noter dans le gestionnaire.',
        replace: 'Un nouveau token classique avec la seule permission `read:packages`, puis `docker login ghcr.io` sur le NAS.',
        doc: { section: '2. Token de lecture pour le NAS', detail: 'et « 6. Premier démarrage »' },
      },
    ],
  },
  {
    id: 'deploiement',
    title: 'Déploiement automatique (secrets GitHub)',
    keys: [
      {
        name: 'Clé SSH de déploiement (`NAS_SSH_KEY`)',
        role: 'Le job `deploy` se connecte au NAS avec elle : commande forcée `deploy.sh`, aucun shell.',
        where: 'Secret GitHub. Sa moitié publique dans `~/.ssh/authorized_keys` de mon compte sur le NAS ; la privée ne reste pas sur le Mac.',
        expiry: NO_EXPIRY,
        replace: "Une nouvelle paire `ssh-keygen`, sa ligne dans `authorized_keys` à la place de l'ancienne, puis `gh secret set NAS_SSH_KEY`.",
        doc: { section: 'Déploiement automatique', detail: 'points 3, 4 et 7' },
      },
      {
        name: 'Client OAuth Tailscale (`TS_OAUTH_CLIENT_ID`, `TS_OAUTH_SECRET`)',
        role: 'Fait entrer le job dans le tailnet en nœud éphémère `tag:ci`, qui ne joint que le port SSH du NAS.',
        where: 'Secrets GitHub, et le gestionnaire de mots de passe.',
        expiry: NO_KNOWN_EXPIRY,
        replace: 'Un nouveau client dans la console Tailscale (Auth Keys en écriture, tag `tag:ci`), puis `gh secret set` pour les deux.',
        doc: { section: 'Déploiement automatique', detail: 'points 1 et 7' },
      },
      {
        name: "Clé d'hôte du NAS (`NAS_KNOWN_HOSTS`)",
        role: 'Épingle la clé SSH du NAS : le job refuse un autre serveur.',
        where: 'Secret GitHub.',
        expiry: "À refaire si le NAS change de clé d'hôte (réinstallation).",
        replace: '`ssh-keyscan` vers le NAS, empreinte comparée sur le NAS, puis `gh secret set NAS_KNOWN_HOSTS`.',
        doc: { section: 'Déploiement automatique', detail: 'point 7' },
      },
      {
        name: 'Adresse du NAS (`NAS_HOST`, `NAS_USER`, `NAS_SSH_PORT`)',
        role: 'Le nom du NAS dans le tailnet, mon compte et le port SSH : pas des clés, mais gardés hors du dépôt.',
        where: 'Secrets GitHub.',
        expiry: NO_EXPIRY,
        replace: '`gh secret set` pour chacun.',
        doc: { section: 'Déploiement automatique', detail: 'point 7' },
      },
    ],
  },
  {
    id: 'extension',
    title: 'Extension Firefox (AMO)',
    keys: [
      {
        name: "Clés d'API AMO (`WEB_EXT_API_KEY`, `WEB_EXT_API_SECRET`)",
        role: "Signent l'extension Twitch par addons.mozilla.org, en canal non listé, sur le Mac seulement.",
        where: "Mon compte AMO (« Gérer les clés d'API »), et le gestionnaire de mots de passe ; jamais dans un fichier, le dépôt ni la CI.",
        expiry: NO_KNOWN_EXPIRY,
        replace: 'De nouvelles clés dans AMO, lues sans écho au moment de signer (`extension/sign.sh`).',
        doc: { section: 'Mise à jour : Twitch', detail: 'étape 2, « Signer »' },
      },
    ],
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

/** « `docs/deploiement.md`, « Mise à jour : YouTube », étape 3. » */
export function docText(doc: DocRef): string {
  return `\`docs/deploiement.md\`, « ${doc.section} »${doc.detail ? `, ${doc.detail}` : ''}.`
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
