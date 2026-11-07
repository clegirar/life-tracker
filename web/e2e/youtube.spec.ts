import { execFileSync } from 'node:child_process'
import { readFileSync } from 'node:fs'
import { expect, test, type APIRequestContext, type Page } from '@playwright/test'
import { addDays, API_URL, apiSession, checkLayout, DESKTOP, fullShot, login, openMenu, parisToday, PASSWORD, send, USER, YOUTUBE_TOKEN } from './helpers'

// YouTube : la carte du jour, la carte de la page d'état et le retour de Google sur `/autorisation`.
// Les vues entrent par la vraie API d'ingestion avec le token de la source `youtube` (web/test.sh), comme
// le collecteur ; la demande d'accord aussi, par `PUT /v1/sources/me/authorization`. Données fictives.
const SHOTS = process.env.E2E_SCREENSHOTS ?? 'e2e-artifacts/screenshots'
const HOUR = 3_600_000
const DATE = addDays(parisToday(), -9)
const BEFORE = addDays(DATE, -20)

/** 43 caractères au moins, comme l'état du connecteur. */
const stateOf = (name: string) => `etatFictif-${name}-${'x'.repeat(40)}`
const consentUrl = (state: string) =>
  `https://accounts.google.com/o/oauth2/v2/auth?client_id=client-fictif&response_type=code&state=${state}`
const CODE = '4/0Fictif-code_abc.def'
/** Une miniature fictive, générée : aucun appel au vrai `i.ytimg.com`. */
const THUMB = readFileSync(new URL('./fixtures/miniature-fictive.png', import.meta.url))

function sql(statement: string) {
  execFileSync('docker', ['exec', process.env.E2E_DB_CONTAINER!, 'psql', '-q', '-U', 'postgres', '-d', 'tracker', '-c', statement])
}

/** Midi passé de quelques minutes, en UTC : l'heure de Paris dépend de la saison, l'ordre non. */
const at = (date: string, hour: number, minute: number) => `${date}T${String(hour).padStart(2, '0')}:${String(minute).padStart(2, '0')}:00.000Z`

interface Video {
  id: string
  title: string | null
  channel: [string, string] | null
  duration: number | null
}

const A: [string, string] = ['UCchaineFictiveA000000', 'Chaîne Fictive A']
const B: [string, string] = ['UCchaineFictiveB000000', 'Chaîne Fictive B']
const PASTA: Video = { id: 'vidFictif01', title: 'Première vidéo fictive : les pâtes', channel: A, duration: 549 }
const LONG: Video = { id: 'vidFictif02', title: 'Deuxième vidéo fictive, la longue', channel: B, duration: 3723 }
const SHORT: Video = { id: 'vidFictif03', title: 'Troisième vidéo fictive', channel: A, duration: 754 }
const GONE: Video = { id: 'vidSupprime', title: null, channel: null, duration: null }

const watch = (video: Video, occurredAt: string) => ({
  key: `${video.id}:${Date.parse(occurredAt)}`,
  kind: 'youtube.watch',
  schema_version: 1,
  occurred_at: occurredAt,
  payload: { video_id: video.id, title: video.title, channel_id: video.channel?.[0] ?? null, channel_name: video.channel?.[1] ?? null },
})

const info = (video: Video) => ({
  key: video.id,
  kind: 'youtube.video_info',
  schema_version: 1,
  occurred_at: new Date().toISOString(),
  payload: { video_id: video.id, duration_s: video.duration, title: video.title, channel_id: video.channel?.[0] ?? null, channel_name: video.channel?.[1] ?? null },
})

async function seed(request: APIRequestContext) {
  await send(
    request,
    [
      // Vue une première fois avant : ni la vidéo ni sa chaîne ne sont des découvertes du jour.
      watch(PASTA, at(BEFORE, 18, 0)),
      watch(PASTA, at(DATE, 18, 30)),
      watch(LONG, at(DATE, 19, 0)),
      watch(SHORT, at(DATE, 19, 40)),
      watch(GONE, at(DATE, 20, 0)),
      info(PASTA),
      info(LONG),
      info(SHORT),
    ],
    YOUTUBE_TOKEN,
  )
  const cookie = await apiSession(request)
  await expect
    .poll(
      async () => {
        const body = await (await request.get(`/v1/days/${DATE}/video`, { headers: { Cookie: cookie } })).json()
        return [body.count, body.seconds, body.without_duration]
      },
      { timeout: 30_000 },
    )
    .toEqual([4, 549 + 3723 + 754, 1])
}

async function report(request: APIRequestContext, status: Record<string, unknown>) {
  const response = await request.put(`${API_URL}/v1/sources/me/status`, {
    headers: { Authorization: `Bearer ${YOUTUBE_TOKEN}` },
    data: status,
  })
  expect(response.status()).toBe(204)
}

/** Comme le connecteur : il confie l'URL de consentement et garde l'état. */
async function requestConsent(request: APIRequestContext, state: string) {
  const response = await request.put(`${API_URL}/v1/sources/me/authorization`, {
    headers: { Authorization: `Bearer ${YOUTUBE_TOKEN}` },
    data: { authorization_url: consentUrl(state), state },
  })
  expect(response.status()).toBe(204)
}

/** Ce que le connecteur vient chercher, une seule fois. */
async function handover(request: APIRequestContext) {
  const response = await request.get(`${API_URL}/v1/sources/me/authorization/code`, {
    headers: { Authorization: `Bearer ${YOUTUBE_TOKEN}` },
  })
  expect(response.status()).toBe(200)
  return response.json()
}

const renewal = (request: APIRequestContext) =>
  report(request, {
    state: 'action_required',
    reason: 'consent_expired',
    expires_at: new Date(Date.now() - HOUR).toISOString(),
    next_run_at: new Date(Date.now() + 20 * HOUR).toISOString(),
    last_collected_at: new Date(Date.now() - 26 * HOUR).toISOString(),
  })

/** Google répond, mais aucun export n'aboutit depuis plus de deux jours. */
const staleExports = (request: APIRequestContext) =>
  report(request, {
    state: 'error',
    reason: 'service_unavailable',
    expires_at: new Date(Date.now() + 5 * 24 * HOUR).toISOString(),
    next_run_at: new Date(Date.now() + 5 * 60_000).toISOString(),
    last_collected_at: new Date(Date.now() - 50 * HOUR).toISOString(),
  })

const card = (page: Page) => page.getByRole('article', { name: 'YouTube' })

/** Les miniatures servies par une image locale ; `missing` répond 404, comme une miniature retirée. */
async function offlineThumbnails(page: Page, missing: string[] = []) {
  await page.route('https://i.ytimg.com/**', (route) =>
    missing.some((id) => route.request().url().includes(`/vi/${id}/`))
      ? route.fulfill({ status: 404 })
      : route.fulfill({ status: 200, contentType: 'image/png', body: THUMB }),
  )
}

async function openStatus(page: Page) {
  await openMenu(page)
  await page.getByRole('link', { name: /^État/ }).click()
  await expect(page.getByRole('heading', { level: 1, name: 'État' })).toBeVisible()
  await expect(page.getByRole('dialog', { name: 'Menu' })).toBeHidden()
}

test.beforeAll(async ({ request }) => {
  await seed(request)
})

// Les autres fichiers retrouvent une source sans état ni demande en cours.
test.afterEach(async ({ request }) => {
  sql("UPDATE core.sources SET status = NULL, status_reported_at = NULL WHERE name = 'youtube'")
  // Une demande en cours se clôt par une réponse, que le connecteur emporte.
  await requestConsent(request, stateOf('fin'))
  const cookie = await apiSession(request)
  await request.post('/v1/authorizations/callback', {
    headers: { Cookie: cookie, 'X-CSRF-Protection': '1' },
    data: { state: stateOf('fin'), error: 'access_denied' },
  })
  await handover(request)
})

test('la carte du jour : vidéos, durée des vidéos, vidéo indisponible et liens', async ({ page }) => {
  await offlineThumbnails(page, [SHORT.id])
  await login(page)
  await page.goto(`/jour/${DATE}`)
  const video = page.getByTestId('video')
  await expect(video.getByRole('heading', { name: 'YouTube' })).toBeVisible()
  await expect(video.getByTestId('video-count')).toHaveText('4')
  // 549 + 3 723 + 754 s : 83,8 min, 1 h 24. La durée des vidéos, pas le temps regardé.
  await expect(video.getByTestId('video-duration')).toHaveText('1 h 24')
  await expect(video).toContainText('Durée des vidéos')
  await expect(video).toContainText('Pas le temps regardé.')
  await expect(video.getByTestId('video-without-duration')).toHaveText('Sans compter 1 vidéo de durée inconnue.')
  await expect(video).toContainText('Chaîne du jourChaîne Fictive A · 2 vidéos')

  const watches = video.getByTestId('video-watch')
  await expect(watches).toHaveCount(4)
  const link = video.getByRole('link', { name: /Deuxième vidéo fictive, la longue/ })
  await expect(link).toHaveAttribute('href', 'https://www.youtube.com/watch?v=vidFictif02')
  await expect(link).toHaveAttribute('target', '_blank')
  await expect(link).toHaveAttribute('rel', 'noopener noreferrer')
  await expect(link).toContainText('1:02:03')
  await expect(watches.nth(0)).toContainText('déjà vue')
  await expect(watches.nth(1)).not.toContainText('déjà vue')
  // Supprimée ou privée : ni titre, ni chaîne, ni lien.
  await expect(watches.nth(3)).toContainText('Vidéo indisponible')
  await expect(watches.nth(3).getByRole('link')).toHaveCount(0)
  await expect(video.getByRole('link')).toHaveCount(3)

  // Une miniature par vidéo disponible, décorative, sans Referer ; aucune pour la vidéo indisponible.
  await watches.nth(3).scrollIntoViewIfNeeded()
  const thumb = watches.nth(1).locator('img')
  await expect(thumb).toHaveAttribute('src', 'https://i.ytimg.com/vi/vidFictif02/mqdefault.jpg')
  await expect(thumb).toHaveAttribute('alt', '')
  await expect(thumb).toHaveAttribute('loading', 'lazy')
  await expect(thumb).toHaveAttribute('referrerpolicy', 'no-referrer')
  await expect.poll(() => thumb.evaluate((img: HTMLImageElement) => img.complete && img.naturalWidth)).toBe(32)
  await expect(watches.nth(0).locator('img')).toHaveAttribute('src', 'https://i.ytimg.com/vi/vidFictif01/mqdefault.jpg')
  await expect(watches.nth(3).getByTestId('video-thumb')).toHaveCount(0)
  await expect(watches.nth(3).locator('img')).toHaveCount(0)
  // Une miniature que Google ne rend pas : l'image s'efface, le cadre 16/9 reste.
  await expect(watches.nth(2).locator('img')).toHaveCount(0)
  const box = await watches.nth(2).getByTestId('video-thumb').boundingBox()
  expect(box && Math.round((box.width / box.height) * 9)).toBe(16)

  // Pas de « Nouvelles chaînes », retirées à ma demande.
  await expect(video.getByTestId('new-channels')).toHaveCount(0)
  await checkLayout(page)

  // Avant l'historique : pas de données, pas « aucune vidéo ».
  await page.goto(`/jour/${addDays(BEFORE, -1)}`)
  await expect(page.getByTestId('video')).toContainText('Pas de données YouTube à cette date.')
  // Entre deux vues reçues : aucune vidéo.
  await page.goto(`/jour/${addDays(DATE, -1)}`)
  await expect(page.getByTestId('video')).toContainText('Aucune vidéo ce jour-là.')
  // Après la dernière vue reçue : l'export est quotidien, on ne sait pas encore.
  await page.goto(`/jour/${addDays(DATE, 1)}`)
  await expect(page.getByTestId('video')).toContainText("Pas encore de données pour ce jour : l'historique YouTube n'arrive qu'une fois par jour.")
})

test("la page d'état propose de renouveler l'accord, par l'URL du connecteur", async ({ page, request }) => {
  await renewal(request)
  await login(page)
  await openStatus(page)
  const youtube = card(page)
  await expect(youtube.getByTestId('status-word')).toHaveText('À faire')
  await expect(youtube).toContainText(/Accès Googleexpiré/)
  await expect(youtube).toContainText(/Prochain export(demain )?à/)
  await expect(youtube).toContainText(/Dernier export(hier|le \d+\s\S+) à \d+\sh\s\d\d/)
  // Sans URL encore : elle arrive à la prochaine passe.
  await expect(youtube.getByRole('link', { name: "Renouveler l'accord" })).toHaveCount(0)
  await expect(youtube).toContainText("Le lien vers Google arrive d'ici quelques minutes")

  await requestConsent(request, stateOf('renouveler'))
  await page.reload()
  const button = youtube.getByRole('link', { name: "Renouveler l'accord" })
  await expect(button).toHaveAttribute('href', consentUrl(stateOf('renouveler')))
  // Dans l'onglet même : Google revient sur /autorisation.
  await expect(button).not.toHaveAttribute('target', /.+/)
  await expect(youtube).toContainText("L'accord donné à Google est à renouveler")
  await checkLayout(page)

  // Un premier accord : « Donner l'accord ».
  await report(request, { state: 'action_required', reason: 'consent_required' })
  await page.reload()
  await expect(youtube.getByRole('link', { name: "Donner l'accord" })).toBeVisible()
})

test("la page d'état dit le dernier export, et quand aucun n'aboutit depuis deux jours", async ({ page, request }) => {
  await report(request, { state: 'ok', last_collected_at: new Date(Date.now() - 3 * HOUR).toISOString() })
  await login(page)
  await openStatus(page)
  const youtube = card(page)
  await expect(youtube.getByTestId('status-word')).toHaveText('Fonctionne')
  await expect(youtube).toContainText('Dernier exportil y a 3\u00a0h')

  await staleExports(request)
  await page.reload()
  await expect(youtube.getByTestId('status-word')).toHaveText('En panne')
  await expect(youtube).toContainText(/Dernier exportle \d+\s\S+ à/)
  await expect(youtube).toContainText("Google répond, mais aucun export n'a abouti depuis 2 jours.")
  await expect(youtube).toContainText('Le collecteur réessaie seul à chaque export permis')
  await expect(youtube).toContainText('sudo docker compose logs collector')
  await checkLayout(page)
})

test("le retour de Google : le code va au connecteur une fois et quitte l'adresse", async ({ page, request }) => {
  await renewal(request)
  await requestConsent(request, stateOf('accord'))
  // Sans session : la connexion ramène à la même adresse, requête comprise.
  const sent: string[] = []
  page.on('request', (r) => {
    if (r.url().endsWith('/v1/authorizations/callback')) sent.push(r.postData() ?? '')
  })
  await page.goto(`/autorisation?state=${stateOf('accord')}&code=${encodeURIComponent(CODE)}&scope=fictif`)
  await expect(page.getByRole('heading', { name: 'Connexion' })).toBeVisible()
  await page.getByLabel('Identifiant').fill(USER)
  await page.getByLabel('Mot de passe').fill(PASSWORD)
  await page.getByRole('button', { name: 'Se connecter' }).click()

  await expect(page.getByTestId('authorization-title')).toHaveText('Accord reçu.')
  await expect(page.getByTestId('authorization')).toContainText('La collecte YouTube reprend dans une minute.')
  expect(new URL(page.url()).pathname).toBe('/autorisation')
  expect(page.url()).not.toContain('code')
  expect(page.url()).not.toContain('state')
  await expect(page.getByTestId('authorization')).not.toContainText('Fictif-code')
  // Une seule fois, exactement l'état et le code.
  expect(sent.map((body) => JSON.parse(body))).toEqual([{ state: stateOf('accord'), code: CODE }])

  // Le connecteur emporte le code, une fois.
  expect(await handover(request)).toEqual({ code: CODE })
  expect(await handover(request)).toEqual({ code: null })

  // Recharger redit l'issue, sans rien renvoyer.
  await page.reload()
  await expect(page.getByTestId('authorization-title')).toHaveText('Accord reçu.')
  expect(sent).toHaveLength(1)

  // La demande est close : plus de bouton sur la page d'état.
  await page.getByRole('link', { name: "Voir l'état" }).click()
  await expect(page.getByRole('heading', { level: 1, name: 'État' })).toBeVisible()
  expect(new URL(page.url()).pathname).toBe('/etat')
  await expect(card(page).getByRole('link', { name: "Renouveler l'accord" })).toHaveCount(0)
})

test("un refus, puis un lien expiré ou incomplet", async ({ page, request }) => {
  await requestConsent(request, stateOf('refus'))
  await login(page)
  await page.goto(`/autorisation?error=access_denied&state=${stateOf('refus')}`)
  await expect(page.getByTestId('authorization-title')).toHaveText("Tu as refusé l'accord.")
  await expect(page.getByRole('link', { name: 'Ouvrir la page État' })).toBeVisible()
  expect(await handover(request)).toEqual({ error: 'access_denied' })

  // L'état n'attend plus rien : le lien a expiré.
  await page.goto(`/autorisation?state=${stateOf('refus')}&code=${encodeURIComponent(CODE)}`)
  await expect(page.getByTestId('authorization-title')).toHaveText('Ce lien a expiré.')
  await expect(page.getByTestId('authorization')).toContainText('Relance depuis la page État.')
  expect(page.url()).not.toContain('code')

  await page.goto('/autorisation')
  await expect(page.getByTestId('authorization-title')).toHaveText('Ce lien a expiré.')
  await checkLayout(page)
  await page.getByRole('link', { name: 'Ouvrir la page État' }).click()
  await expect(page.getByRole('heading', { level: 1, name: 'État' })).toBeVisible()
})

for (const colorScheme of ['light', 'dark'] as const) {
  test.describe(colorScheme, () => {
    test.use({ colorScheme })

    for (const [width, viewport] of [
      [390, null],
      [1280, DESKTOP],
    ] as const) {
      test.describe(`${width} px`, () => {
        if (viewport) test.use({ viewport, isMobile: false, hasTouch: false })

        test(`YouTube à ${width} px, ${colorScheme}`, async ({ page, request }) => {
          const shot = (name: string) => fullShot(page, `${SHOTS}/${width}-${colorScheme}-${name}.png`)
          await offlineThumbnails(page)
          await login(page)
          await page.goto(`/jour/${DATE}`)
          await expect(page.getByTestId('video-count')).toHaveText('4')
          await expect(page.getByTestId('music')).not.toContainText('Chargement')
          await expect(page.getByTestId('activity')).not.toContainText('Chargement')
          // Les miniatures arrivées, pour la capture : elles sont chargées paresseusement.
          const thumbs = page.getByTestId('video').locator('img')
          await expect(thumbs).toHaveCount(3)
          for (const img of await thumbs.all()) {
            await img.scrollIntoViewIfNeeded()
            await expect.poll(() => img.evaluate((el: HTMLImageElement) => el.complete && el.naturalWidth > 0)).toBe(true)
          }
          await checkLayout(page)
          await shot('youtube-jour')

          await renewal(request)
          await requestConsent(request, stateOf('capture'))
          await openStatus(page)
          await expect(card(page).getByRole('link', { name: "Renouveler l'accord" })).toBeVisible()
          await checkLayout(page)
          await shot('youtube-etat')

          await staleExports(request)
          await page.reload()
          await expect(card(page).getByTestId('status-word')).toHaveText('En panne')
          await checkLayout(page)
          await shot('youtube-etat-export')

          await page.goto(`/autorisation?state=${stateOf('capture')}&code=${encodeURIComponent(CODE)}`)
          await expect(page.getByTestId('authorization-title')).toHaveText('Accord reçu.')
          await checkLayout(page)
          await shot('youtube-autorisation')
          await handover(request)
        })
      })
    }
  })
}
