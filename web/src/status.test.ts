import { describe, expect, it } from 'vitest'
import type { SourceStatus } from './api'
import { cardState, codeParts, currentStatus, metricLine, since, sourceCard, thresholdText, until } from './status'

// Le 5 nov. 2026, 14 h 00 à Paris (heure d'hiver, UTC+1).
const NOW = Date.parse('2026-11-05T13:00:00Z')
const NBSP = ' '
const iso = (offsetMs: number) => new Date(NOW + offsetMs).toISOString()
const MIN = 60_000
const HOUR = 3_600_000
const DAY = 86_400_000

function source(overrides: Partial<SourceStatus> = {}): SourceStatus {
  return {
    name: 'garmin',
    label: 'Garmin Connect',
    last_seen_at: iso(-10 * MIN),
    silence_threshold_seconds: 43_200,
    silent: false,
    status: { state: 'ok', reason: null, expires_at: iso(300 * DAY), next_run_at: iso(5 * MIN) },
    status_reported_at: iso(-10 * MIN),
    metrics: [],
    authorization_url: null,
    ...overrides,
  }
}

describe('les heures relatives', () => {
  it('dit le passé simplement, puis le jour et l’heure', () => {
    expect(since(NOW - 20_000, NOW)).toBe("à l'instant")
    expect(since(NOW - 10 * MIN, NOW)).toBe(`il y a 10${NBSP}min`)
    expect(since(NOW - 2 * HOUR - 5 * MIN, NOW)).toBe(`il y a 2${NBSP}h`)
    expect(since(Date.parse('2026-11-05T06:05:00Z'), NOW)).toBe(`aujourd'hui à 7${NBSP}h${NBSP}05`)
    expect(since(Date.parse('2026-11-04T19:30:00Z'), NOW)).toBe(`hier à 20${NBSP}h${NBSP}30`)
    expect(since(Date.parse('2026-11-01T19:30:00Z'), NOW)).toBe(`le 1${NBSP}nov. à 20${NBSP}h${NBSP}30`)
    expect(since(Date.parse('2025-12-24T19:30:00Z'), NOW)).toBe(`le 24${NBSP}déc. 2025 à 20${NBSP}h${NBSP}30`)
  })

  it('dit le futur en heure de Paris', () => {
    expect(until(Date.parse('2026-11-05T13:05:00Z'), NOW)).toBe(`à 14${NBSP}h${NBSP}05`)
    expect(until(Date.parse('2026-11-06T06:00:00Z'), NOW)).toBe(`demain à 7${NBSP}h${NBSP}00`)
  })

  it('passe minuit à Paris, pas en UTC', () => {
    // 23 h 30 UTC le 4 : déjà le 5 à Paris.
    expect(since(Date.parse('2026-11-04T23:30:00Z'), NOW)).toBe(`aujourd'hui à 0${NBSP}h${NBSP}30`)
  })
})

describe("l'état d'une carte", () => {
  it('fonctionne par défaut', () => {
    expect(cardState(source(), NOW)).toBe('ok')
    expect(cardState(source({ status: null, silence_threshold_seconds: null }), NOW)).toBe('ok')
  })

  it('est en panne sur une erreur rapportée, avec sa raison', () => {
    const broken = source({
      last_seen_at: iso(-3 * HOUR),
      status: { state: 'error', reason: 'service_unavailable', expires_at: null, next_run_at: null },
      status_reported_at: iso(-5 * MIN),
    })
    expect(cardState(broken, NOW)).toBe('broken')
    expect(sourceCard(broken, NOW).guidance?.title).toBe('Le service ne répond pas, ou refuse la demande.')
  })

  it("oublie une erreur suivie d'un contact réussi, sauf si la source est muette", () => {
    const stale = source({
      last_seen_at: iso(-1 * MIN),
      status: { state: 'error', reason: 'rate_limited', expires_at: iso(300 * DAY), next_run_at: null },
      status_reported_at: iso(-20 * MIN),
    })
    expect(cardState(stale, NOW)).toBe('ok')
    expect(currentStatus(stale)?.expires_at).toBe(iso(300 * DAY))
    expect(cardState({ ...stale, silent: true }, NOW)).toBe('broken')
  })

  it('demande une action sur une clé refusée, même muette', () => {
    const card = sourceCard(
      source({
        silent: true,
        status: { state: 'action_required', reason: 'key_rejected', expires_at: null, next_run_at: null },
      }),
      NOW,
    )
    expect(card.state).toBe('todo')
    expect(card.guidance?.title).toBe('Garmin refuse la clé : elle a expiré ou a été révoquée.')
    expect(card.guidance?.steps.join(' ')).toContain('GARMIN_KEY_EXPIRES_ON')
    expect(card.guidance?.note).toContain('Mise à jour : Garmin Connect')
  })

  it('donne une marche générique pour une autre clé refusée', () => {
    const card = sourceCard(
      source({
        name: 'lastfm',
        label: 'Musique',
        status: { state: 'action_required', reason: 'key_rejected', expires_at: null, next_run_at: null },
      }),
      NOW,
    )
    expect(card.guidance?.title).toBe('Le service refuse la clé du collecteur : elle a expiré ou a été révoquée.')
    expect(card.guidance?.steps.join(' ')).toContain('LASTFM_API_KEY')
    expect(card.guidance?.steps.join(' ')).not.toContain('Garmin')
  })

  it('prévient quatorze jours avant l’échéance, et après', () => {
    // Fin du 12 nov. à Paris : minuit du 13, heure de Paris.
    const soon = source({ status: { state: 'ok', reason: null, expires_at: '2026-11-12T23:00:00Z', next_run_at: null } })
    const card = sourceCard(soon, NOW)
    expect(card.state).toBe('todo')
    expect(card.lines).toContainEqual({ label: 'Clé', value: `expire le 12${NBSP}nov., dans 7 jours`, warn: 'soon' })
    expect(card.guidance?.title).toBe(`La clé expire le 12${NBSP}nov. : remplace-la avant.`)

    const far = sourceCard(source({ status: { state: 'ok', reason: null, expires_at: '2027-11-04T23:00:00Z', next_run_at: null } }), NOW)
    expect(far.state).toBe('ok')
    expect(far.lines).toContainEqual({ label: 'Clé', value: `valable jusqu'au 4${NBSP}nov. 2027` })
    expect(far.guidance).toBeNull()

    const past = sourceCard(source({ status: { state: 'ok', reason: null, expires_at: '2026-11-03T23:00:00Z', next_run_at: null } }), NOW)
    expect(past.state).toBe('todo')
    expect(past.lines).toContainEqual({ label: 'Clé', value: `expirée le 3${NBSP}nov.`, warn: 'past' })
    expect(past.guidance?.title).toBe(`La clé a expiré le 3${NBSP}nov. : remplace-la.`)
  })

  it('est silencieuse au-delà du seuil', () => {
    const card = sourceCard(source({ name: 'lastfm', label: 'Musique', silence_threshold_seconds: 21_600, silent: true, status: null }), NOW)
    expect(card.state).toBe('silent')
    expect(card.guidance?.title).toBe(`Aucun contact réussi avec Last.fm depuis plus de 6${NBSP}h : des données peuvent manquer.`)
  })

  it('traduit chaque raison', () => {
    for (const reason of ['consent_required', 'consent_expired', 'profile_private', 'rate_limited', 'service_unavailable', 'ingest_failed'] as const) {
      const card = sourceCard(source({ status: { state: 'error', reason, expires_at: null, next_run_at: null } }), NOW)
      expect(card.guidance?.title, reason).toBeTruthy()
    }
  })
})

describe('le collecteur Steam', () => {
  const steam = (overrides: Partial<SourceStatus> = {}) =>
    source({
      name: 'steam',
      label: 'Steam',
      silence_threshold_seconds: 21_600,
      status: { state: 'ok', reason: null, expires_at: null, next_run_at: iso(10 * MIN), last_collected_at: iso(-5 * MIN) },
      metrics: [
        { metric: 'game.play', label: 'Jeux', last_at: iso(-2 * HOUR) },
        { metric: 'game.achievement', label: 'Succès', last_at: iso(-3 * HOUR) },
      ],
      ...overrides,
    })

  it('dit la dernière partie et le dernier succès, sans échéance de clé', () => {
    const card = sourceCard(steam(), NOW)
    expect(card.state).toBe('ok')
    expect(card.lines).toContainEqual({ label: 'Dernière partie', value: `il y a 2${NBSP}h` })
    expect(card.lines).toContainEqual({ label: 'Dernier succès', value: `il y a 3${NBSP}h` })
    expect(card.lines.map((line) => line.label)).not.toContain('Clé')
  })

  it('à faire quand le profil est privé, même sans collecte réussie', () => {
    const card = sourceCard(
      steam({ status: { state: 'action_required', reason: 'profile_private', expires_at: null, next_run_at: null, last_collected_at: null } }),
      NOW,
    )
    expect(card.state).toBe('todo')
    expect(card.guidance?.title).toBe('Le profil Steam est privé : Steam ne donne ni les jeux ni les succès.')
    expect(card.guidance?.steps.join(' ')).toContain('« Détails des jeux »')
    expect(card.lines.map((line) => line.label)).not.toContain('Dernière collecte')
  })

  it('une clé refusée renvoie vers `STEAM_API_KEY`', () => {
    const card = sourceCard(steam({ status: { state: 'action_required', reason: 'key_rejected', expires_at: null, next_run_at: null } }), NOW)
    expect(card.state).toBe('todo')
    expect(card.guidance?.title).toMatch(/^Steam refuse la clé/)
    expect(card.guidance?.steps.join(' ')).toContain('`STEAM_API_KEY`')
  })

  it('silencieux, renvoie vers le collecteur du NAS', () => {
    const card = sourceCard(steam({ silent: true, last_seen_at: iso(-8 * HOUR) }), NOW)
    expect(card.state).toBe('silent')
    expect(card.guidance?.title).toBe(`Aucun contact réussi avec Steam depuis plus de 6${NBSP}h : des données peuvent manquer.`)
    expect(card.guidance?.steps.join(' ')).toContain('sudo docker compose ps collector')
  })
})

describe("l'accord à donner (YouTube)", () => {
  const URL = 'https://accounts.google.com/o/oauth2/v2/auth?client_id=x&state=abc'
  const youtube = (overrides: Partial<SourceStatus> = {}) =>
    source({
      name: 'youtube',
      label: 'YouTube',
      silence_threshold_seconds: 259_200,
      status: { state: 'ok', reason: null, expires_at: iso(5 * DAY), next_run_at: iso(20 * HOUR) },
      ...overrides,
    })

  it('fonctionne avec un jeton de 7 jours : prévenir 14 jours avant la rendrait toujours à faire', () => {
    const card = sourceCard(youtube(), NOW)
    expect(card.state).toBe('ok')
    expect(card.lines).toContainEqual({ label: 'Accès Google', value: `valable jusqu'au 10${NBSP}nov. à 14${NBSP}h${NBSP}00` })
    expect(card.lines).toContainEqual({ label: 'Prochain export', value: `demain à 10${NBSP}h${NBSP}00` })
  })

  it("colore l'échéance deux jours avant, sans changer l'état", () => {
    const card = sourceCard(youtube({ status: { state: 'ok', reason: null, expires_at: iso(30 * HOUR), next_run_at: null } }), NOW)
    expect(card.state).toBe('ok')
    expect(card.lines).toContainEqual({ label: 'Accès Google', value: `expire demain à 20${NBSP}h${NBSP}00`, warn: 'soon' })
    const past = sourceCard(youtube({ status: { state: 'ok', reason: null, expires_at: iso(-2 * HOUR), next_run_at: null } }), NOW)
    expect(past.lines).toContainEqual({ label: 'Accès Google', value: `expiré aujourd'hui à 12${NBSP}h${NBSP}00`, warn: 'past' })
  })

  it('propose de renouveler, par l’URL de la demande', () => {
    const card = sourceCard(
      youtube({ status: { state: 'action_required', reason: 'consent_expired', expires_at: iso(-HOUR), next_run_at: null }, authorization_url: URL }),
      NOW,
    )
    expect(card.state).toBe('todo')
    expect(card.guidance?.action).toEqual({ label: "Renouveler l'accord", href: URL })
    expect(card.guidance?.title).toMatch(/à renouveler/)
  })

  it('propose de donner un premier accord', () => {
    const card = sourceCard(
      youtube({ status: { state: 'action_required', reason: 'consent_required', expires_at: null, next_run_at: null }, authorization_url: URL }),
      NOW,
    )
    expect(card.guidance?.action).toEqual({ label: "Donner l'accord", href: URL })
    expect(card.guidance?.title).toBe('La collecte attend ton accord pour commencer.')
  })

  it("sans URL encore, dit qu'elle arrive", () => {
    const card = sourceCard(youtube({ status: { state: 'action_required', reason: 'consent_expired', expires_at: null, next_run_at: null } }), NOW)
    expect(card.state).toBe('todo')
    expect(card.guidance?.action).toBeUndefined()
    expect(card.guidance?.note).toMatch(/arrive d'ici quelques minutes/)
  })

  it('dit le dernier export, même avant le premier', () => {
    const card = sourceCard(
      youtube({ status: { state: 'ok', reason: null, expires_at: null, next_run_at: null, last_collected_at: iso(-3 * HOUR) } }),
      NOW,
    )
    expect(card.lines[1]).toEqual({ label: 'Dernier export', value: `il y a 3${NBSP}h` })
    const never = sourceCard(youtube({ status: { state: 'ok', reason: null, expires_at: null, next_run_at: null, last_collected_at: null } }), NOW)
    expect(never.lines).toContainEqual({ label: 'Dernier export', value: "aucun pour l'instant" })
    // Un état rapporté avant ce champ : la ligne dit aussi qu'il n'y en a pas.
    expect(sourceCard(youtube(), NOW).lines).toContainEqual({ label: 'Dernier export', value: "aucun pour l'instant" })
  })

  it("en panne sans export depuis deux jours, dit que Google répond et qu'il réessaie seul", () => {
    const card = sourceCard(
      youtube({
        status: { state: 'error', reason: 'service_unavailable', expires_at: iso(5 * DAY), next_run_at: iso(5 * MIN), last_collected_at: iso(-50 * HOUR) },
        status_reported_at: iso(-MIN),
        last_seen_at: iso(-2 * MIN),
      }),
      NOW,
    )
    expect(card.state).toBe('broken')
    expect(card.lines).toContainEqual({ label: 'Dernier export', value: `le 3${NBSP}nov. à 12${NBSP}h${NBSP}00` })
    expect(card.guidance?.title).toBe("Google répond, mais aucun export n'a abouti depuis 2 jours.")
    expect(card.guidance?.note).toMatch(/réessaie seul.*logs sur le NAS/)
    // Une panne de Google avec un export récent reste une panne ordinaire.
    const outage = sourceCard(
      youtube({
        status: { state: 'error', reason: 'service_unavailable', expires_at: null, next_run_at: null, last_collected_at: iso(-3 * HOUR) },
        last_seen_at: iso(-3 * HOUR),
      }),
      NOW,
    )
    expect(outage.guidance?.title).toBe('Le service ne répond pas, ou refuse la demande.')
  })

  it('dit la dernière collecte d’une autre source qui la rapporte', () => {
    const card = sourceCard(
      source({ status: { state: 'ok', reason: null, expires_at: null, next_run_at: null, last_collected_at: iso(-20 * MIN) } }),
      NOW,
    )
    expect(card.lines).toContainEqual({ label: 'Dernière collecte', value: `il y a 20${NBSP}min` })
    expect(sourceCard(source(), NOW).lines.map((line) => line.label)).not.toContain('Dernière collecte')
  })

  it('le bouton suit la demande, quelle que soit la source', () => {
    const card = sourceCard(source({ authorization_url: URL }), NOW)
    expect(card.state).toBe('todo')
    expect(card.guidance?.action?.href).toBe(URL)
  })
})

describe("l'iPhone (Overland)", () => {
  const iphone = (overrides: Partial<SourceStatus> = {}) =>
    source({
      name: 'iphone',
      label: 'Position (iPhone)',
      silence_threshold_seconds: 172_800,
      status: null,
      status_reported_at: null,
      metrics: [
        { metric: 'location.point', label: 'Position', last_at: iso(-15 * MIN) },
        { metric: 'place.visit', label: 'Lieux', last_at: iso(-3 * HOUR) },
      ],
      ...overrides,
    })

  it("dit le dernier envoi de l'iPhone, la dernière position et le dernier lieu, sans passe", () => {
    const card = sourceCard(iphone(), NOW)
    expect(card.state).toBe('ok')
    expect(card.title).toBe('Position (iPhone)')
    expect(card.lines).toEqual([
      { label: "Dernier envoi de l'iPhone", value: `il y a 10${NBSP}min` },
      { label: 'Dernière position', value: `il y a 15${NBSP}min` },
      { label: 'Dernier lieu', value: `il y a 3${NBSP}h` },
    ])
    expect(sourceCard(iphone({ last_seen_at: null, metrics: [] }), NOW).lines).toEqual([{ label: "Dernier envoi de l'iPhone", value: "aucun pour l'instant" }])
    expect(card.guidance).toBeNull()
  })

  it('silencieux, renvoie vers Tailscale, Overland et la localisation « Toujours », pas vers le NAS', () => {
    const card = sourceCard(iphone({ name: 'iphone-2', silent: true, last_seen_at: iso(-3 * DAY) }), NOW)
    expect(card.state).toBe('silent')
    expect(card.guidance?.title).toBe(`Aucun envoi de l'iPhone depuis plus de 2${NBSP}jours : des déplacements peuvent manquer.`)
    const steps = card.guidance?.steps.join(' ') ?? ''
    expect(steps).toContain('Tailscale')
    expect(steps).toContain('Overland')
    expect(steps).toContain('« Toujours »')
    expect(steps).toContain('401')
    expect(steps).not.toContain('collector')
    expect(card.guidance?.note).toContain("Rien n'est perdu")
  })
})

describe("l'extension Twitch", () => {
  const twitch = (overrides: Partial<SourceStatus> = {}) =>
    source({
      name: 'twitch',
      label: 'Twitch',
      silence_threshold_seconds: 259_200,
      status: { state: 'ok', reason: null, expires_at: null, next_run_at: null, last_collected_at: iso(-40 * MIN) },
      metrics: [{ metric: 'stream.watch', label: 'Twitch', last_at: iso(-2 * HOUR) }],
      ...overrides,
    })

  it('dit le dernier signal de Firefox, le dernier envoi et le dernier visionnage', () => {
    const card = sourceCard(twitch(), NOW)
    expect(card.state).toBe('ok')
    expect(card.lines).toEqual([
      { label: 'Dernier signal de Firefox', value: `il y a 10${NBSP}min` },
      { label: 'Dernier envoi', value: `il y a 40${NBSP}min` },
      { label: 'Dernier visionnage', value: `il y a 2${NBSP}h` },
    ])
    expect(sourceCard(twitch({ last_seen_at: null, status: null, metrics: [] }), NOW).lines).toEqual([
      { label: 'Dernier signal de Firefox', value: "aucun pour l'instant" },
    ])
  })

  it('silencieuse, dit le seuil en jours et renvoie vers Firefox, pas vers le NAS', () => {
    const card = sourceCard(twitch({ silent: true, last_seen_at: iso(-4 * DAY) }), NOW)
    expect(card.state).toBe('silent')
    expect(card.guidance?.title).toBe(`Aucun signal de Firefox depuis plus de 3${NBSP}jours : des streams peuvent manquer.`)
    const steps = card.guidance?.steps.join(' ') ?? ''
    expect(steps).toContain('Ouvre Firefox sur le Mac')
    expect(steps).toContain('`about:addons`')
    expect(steps).toContain("page d'options")
    expect(steps).toContain('token est refusé')
    expect(steps).not.toContain('collector')
  })

  it("en panne quand l'envoi échoue, avec la marche de l'extension", () => {
    const card = sourceCard(
      twitch({ status: { state: 'error', reason: 'ingest_failed', expires_at: null, next_run_at: null }, status_reported_at: iso(-5 * MIN) }),
      NOW,
    )
    expect(card.state).toBe('broken')
    expect(card.guidance?.title).toBe("L'extension n'arrive pas à remettre ses segments au serveur.")
    expect(card.guidance?.steps.join(' ')).toContain('sudo docker compose ps backend')
    expect(card.guidance?.note).toContain("Rien n'est perdu")
  })

  it('dit un seuil de 48 h ou plus en jours', () => {
    expect(thresholdText(21_600)).toBe(`6${NBSP}h`)
    expect(thresholdText(43_200)).toBe(`12${NBSP}h`)
    expect(thresholdText(86_400)).toBe(`24${NBSP}h`)
    expect(thresholdText(172_800)).toBe(`2${NBSP}jours`)
    expect(thresholdText(259_200)).toBe(`3${NBSP}jours`)
    expect(thresholdText(270_000)).toBe(`75${NBSP}h`)
  })
})

describe('les lignes', () => {
  it('dit la dernière passe, la prochaine et les dernières données', () => {
    const card = sourceCard(
      source({
        metrics: [
          { metric: 'steps', label: 'Pas', last_at: '2026-11-05T11:00:00Z' },
          { metric: 'sleep', label: 'Sommeil', last_at: '2026-11-04T11:00:00Z' },
          { metric: 'workout', label: 'Séances', last_at: iso(-2 * HOUR) },
        ],
      }),
      NOW,
    )
    expect(card.title).toBe('Garmin Connect')
    expect(card.lines).toEqual([
      { label: 'Dernière passe', value: `il y a 10${NBSP}min` },
      { label: 'Prochaine passe', value: `à 14${NBSP}h${NBSP}05` },
      { label: 'Clé', value: `valable jusqu'au 1${NBSP}sept. 2027` },
      { label: 'Pas', value: "aujourd'hui" },
      { label: 'Sommeil', value: 'hier' },
      { label: 'Dernière séance', value: `il y a 2${NBSP}h` },
    ])
  })

  it('montre le nom sans libellé, un contact sans connecteur', () => {
    const card = sourceCard(
      source({ name: 'ios', label: null, silence_threshold_seconds: null, status: null, status_reported_at: null, last_seen_at: null }),
      NOW,
    )
    expect(card.title).toBe('ios')
    expect(card.lines[0]).toEqual({ label: 'Dernier contact', value: "aucun pour l'instant" })
  })

  it('nomme les métriques connues, et les autres par leur libellé', () => {
    expect(metricLine({ metric: 'coffee', label: 'Cafés', last_at: iso(-3 * HOUR) }, NOW).label).toBe('Dernier café')
    expect(metricLine({ metric: 'music.listen', label: 'Écoutes', last_at: iso(-3 * HOUR) }, NOW).label).toBe('Dernière écoute')
    expect(metricLine({ metric: 'tea', label: 'Thés', last_at: iso(-3 * HOUR) }, NOW).label).toBe('Dernière donnée, thés')
  })

  it("ne montre pas une prochaine passe dépassée", () => {
    const card = sourceCard(source({ status: { state: 'ok', reason: null, expires_at: null, next_run_at: iso(-MIN) } }), NOW)
    expect(card.lines.map((line) => line.label)).not.toContain('Prochaine passe')
  })

  it('signale des écoutes qui n’arrivent plus, sans changer l’état', () => {
    const card = sourceCard(
      source({ name: 'lastfm', label: 'Musique', status: null, metrics: [{ metric: 'music.listen', label: 'Écoutes', last_at: iso(-4 * DAY) }] }),
      NOW,
    )
    expect(card.state).toBe('ok')
    expect(card.hint).toContain('scrobbler')
  })
})

describe('la page', () => {
  it('sépare le code du texte', () => {
    expect(codeParts('Relance : `sudo docker compose up -d collector`.')).toEqual([
      { code: false, text: 'Relance : ' },
      { code: true, text: 'sudo docker compose up -d collector' },
      { code: false, text: '.' },
    ])
  })
})
