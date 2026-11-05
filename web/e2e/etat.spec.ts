import { execFileSync } from 'node:child_process'
import { expect, test, type APIRequestContext, type Page } from '@playwright/test'
import { API_URL, checkLayout, DESKTOP, fullShot, GARMIN_TOKEN, LASTFM_TOKEN, login, openMenu, PASSWORD, USER } from './helpers'

// La page d'état des trackers. Les états entrent comme ceux des connecteurs, par la vraie route
// `PUT /v1/sources/me/status` et le token de la source ; seul le silence se simule dans la base jetable.
// Les libellés (« Garmin Connect », « Musique », « Raccourci café ») sont posés par web/test.sh.
const SHOTS = process.env.E2E_SCREENSHOTS ?? 'e2e-artifacts/screenshots'
const DAY = 86_400_000
const path = (url: string) => new URL(url).pathname

/** Seulement pour simuler un silence ou effacer un état : la base jetable de web/test.sh. */
function sql(statement: string) {
  execFileSync('docker', ['exec', process.env.E2E_DB_CONTAINER!, 'psql', '-q', '-U', 'postgres', '-d', 'tracker', '-c', statement])
}

interface Status {
  state: 'ok' | 'action_required' | 'error'
  reason?: string | null
  expires_at?: string | null
  next_run_at?: string | null
}

async function report(request: APIRequestContext, token: string, status: Status) {
  const response = await request.put(`${API_URL}/v1/sources/me/status`, {
    headers: { Authorization: `Bearer ${token}` },
    data: status,
  })
  expect(response.status()).toBe(204)
}

async function heartbeat(request: APIRequestContext, token: string) {
  const response = await request.post(`${API_URL}/v1/heartbeat`, { headers: { Authorization: `Bearer ${token}` } })
  expect(response.status()).toBe(200)
}

const inDays = (days: number) => new Date(Date.now() + days * DAY).toISOString()
const inMinutes = (minutes: number) => new Date(Date.now() + minutes * 60_000).toISOString()

/** Les autres fichiers retrouvent des sources sans état ni silence. */
async function restore(request: APIRequestContext) {
  sql("UPDATE core.sources SET status = NULL, status_reported_at = NULL WHERE name IN ('garmin', 'lastfm')")
  await heartbeat(request, LASTFM_TOKEN)
  await heartbeat(request, GARMIN_TOKEN)
}

const card = (page: Page, title: string) => page.getByRole('article', { name: title })

async function openStatus(page: Page) {
  await openMenu(page)
  await page.getByRole('link', { name: /^État/ }).click()
  await expect(page.getByRole('heading', { level: 1, name: 'État' })).toBeVisible()
  // Sur iPhone, le tiroir a fini de se refermer.
  await expect(page.getByRole('dialog', { name: 'Menu' })).toBeHidden()
  expect(path(page.url())).toBe('/etat')
}

test.afterEach(async ({ request }) => {
  await restore(request)
})

test('la page exige la session, et sa lecture aussi', async ({ page, request }) => {
  expect((await request.get('/v1/sources/status')).status()).toBe(401)
  await page.goto('/etat')
  await expect(page.getByRole('heading', { name: 'Connexion' })).toBeVisible()
  await page.getByLabel('Identifiant').fill(USER)
  await page.getByLabel('Mot de passe').fill(PASSWORD)
  await page.getByRole('button', { name: 'Se connecter' }).click()
  await expect(page.getByRole('heading', { level: 1, name: 'État' })).toBeVisible()
  expect(path(page.url())).toBe('/etat')
})

test('une carte par tracker : fonctionne, à faire, en panne, silencieux, et le point du menu', async ({ page, request }) => {
  await report(request, GARMIN_TOKEN, { state: 'ok', expires_at: inDays(300), next_run_at: inMinutes(12) })
  await heartbeat(request, GARMIN_TOKEN)
  await heartbeat(request, LASTFM_TOKEN)
  await login(page)
  await expect(page.getByTestId('menu-dot')).toHaveCount(0)
  await openStatus(page)
  await expect(page.getByTestId('status-dot')).toHaveCount(0)
  await expect(page.getByTestId('status-summary')).toHaveText('Tout fonctionne')

  const garmin = card(page, 'Garmin Connect')
  await expect(garmin.getByTestId('status-word')).toHaveText('Fonctionne')
  await expect(garmin).toContainText("Dernière passeà l'instant")
  await expect(garmin).toContainText(/Prochaine passeà \d+\sh\s\d\d/)
  await expect(garmin).toContainText(/Clévalable jusqu'au \d+\s\S+\s\d{4}/)
  await expect(garmin.getByRole('heading', { name: 'Que faire' })).toHaveCount(0)
  await expect(card(page, 'Musique').getByTestId('status-word')).toHaveText('Fonctionne')
  const coffee = card(page, 'Raccourci café')
  await expect(coffee.getByTestId('status-word')).toHaveText('Fonctionne')
  await expect(coffee).toContainText('Dernier contact')
  // Ni l'import Apple ni la web app n'ont de carte.
  await expect(page.getByRole('article')).toHaveCount(3)
  await checkLayout(page)

  // Clé refusée : à faire, avec la marche à suivre, et le point du menu.
  await report(request, GARMIN_TOKEN, { state: 'action_required', reason: 'key_rejected', expires_at: inDays(300) })
  await page.reload()
  await expect(garmin.getByTestId('status-word')).toHaveText('À faire')
  await expect(garmin.getByRole('heading', { name: 'Que faire' })).toBeVisible()
  await expect(garmin).toContainText('Garmin refuse la clé : elle a expiré ou a été révoquée.')
  await expect(garmin).toContainText('GARMIN_KEY_EXPIRES_ON')
  await expect(page.getByTestId('status-summary')).toHaveText('Un tracker attend quelque chose de toi')
  await expect(page.getByTestId('menu-dot')).toBeVisible()
  await openMenu(page)
  await expect(page.getByTestId('status-dot')).toBeVisible()
  await expect(page.getByRole('link', { name: 'État, une action attendue' })).toHaveAttribute('aria-current', 'page')
  await page.keyboard.press('Escape')

  // Échéance dans moins de quatorze jours : à faire aussi.
  await report(request, GARMIN_TOKEN, { state: 'ok', expires_at: inDays(5) })
  await page.reload()
  await expect(garmin.getByTestId('status-word')).toHaveText('À faire')
  await expect(garmin).toContainText(/Cléexpire le \d+\s\S+, dans [45] jours/)
  await expect(garmin).toContainText('remplace-la avant.')

  // Une erreur : en panne, puis oubliée dès qu'une passe réussit.
  await report(request, GARMIN_TOKEN, { state: 'error', reason: 'service_unavailable' })
  await page.reload()
  await expect(garmin.getByTestId('status-word')).toHaveText('En panne')
  await expect(garmin).toContainText('Le collecteur réessaie seul')
  await heartbeat(request, GARMIN_TOKEN)
  await page.reload()
  await expect(garmin.getByTestId('status-word')).toHaveText('Fonctionne')
  await expect(page.getByTestId('menu-dot')).toHaveCount(0)

  // Plus de contact depuis le seuil : silencieux, sans point (rien à faire tant que ça se résout seul).
  sql("UPDATE core.sources SET last_seen_at = now() - interval '2 days' WHERE name = 'lastfm'")
  await page.reload()
  const music = card(page, 'Musique')
  await expect(music.getByTestId('status-word')).toHaveText('Silencieux')
  await expect(music).toContainText('Aucun contact réussi avec Last.fm depuis plus de 6 h')
  await expect(page.getByTestId('status-summary')).toHaveText('Un tracker silencieux')
  await expect(page.getByTestId('menu-dot')).toHaveCount(0)
})

for (const colorScheme of ['light', 'dark'] as const) {
  test.describe(colorScheme, () => {
    test.use({ colorScheme })

    /** Les captures : Garmin à faire, la musique silencieuse, le café qui fonctionne. */
    async function scene(request: APIRequestContext) {
      await report(request, GARMIN_TOKEN, {
        state: 'action_required',
        reason: 'key_rejected',
        expires_at: inDays(300),
        next_run_at: inMinutes(14),
      })
      sql("UPDATE core.sources SET last_seen_at = now() - interval '2 days' WHERE name = 'lastfm'")
    }

    test(`page d'état à 390 px, ${colorScheme}`, async ({ page, request }) => {
      await scene(request)
      await login(page)
      await openStatus(page)
      await expect(card(page, 'Garmin Connect').getByTestId('status-word')).toHaveText('À faire')
      await checkLayout(page)
      await page.screenshot({ path: `${SHOTS}/${colorScheme}-etat.png`, fullPage: true })
      await openMenu(page)
      await page.waitForTimeout(350) // la fin de l'ouverture
      await page.screenshot({ path: `${SHOTS}/${colorScheme}-etat-tiroir.png` })
    })

    test.describe('écran large', () => {
      test.use({ viewport: DESKTOP, isMobile: false, hasTouch: false })

      test(`page d'état à 1280 px, ${colorScheme}`, async ({ page, request }) => {
        await scene(request)
        await login(page)
        await openStatus(page)
        await expect(page.getByRole('complementary', { name: 'Menu' }).getByTestId('status-dot')).toBeVisible()
        await expect(card(page, 'Musique').getByTestId('status-word')).toHaveText('Silencieux')
        await checkLayout(page)
        await fullShot(page, `${SHOTS}/${colorScheme}-large-etat.png`)
      })
    })
  })
}
