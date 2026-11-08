import { execFileSync } from 'node:child_process'
import { expect, test, type APIRequestContext, type Page } from '@playwright/test'
import {
  API_URL,
  APPLE_TOKEN,
  checkLayout,
  DESKTOP,
  fullShot,
  GARMIN_TOKEN,
  IOS_TOKEN,
  IPHONE_TOKEN,
  LASTFM_TOKEN,
  login,
  openMenu,
  PASSWORD,
  STEAM_TOKEN,
  TWITCH_TOKEN,
  USER,
  YOUTUBE_TOKEN,
} from './helpers'

// La page Sources : les fiches des trackers, les autres clés, et les échéances que rapportent les
// sources par la vraie route `PUT /v1/sources/me/status`, comme dans etat.spec.ts. Données fictives.
const SHOTS = process.env.E2E_SCREENSHOTS ?? 'e2e-artifacts/screenshots'
const DAY = 86_400_000
const path = (url: string) => new URL(url).pathname
const inDays = (days: number) => new Date(Date.now() + days * DAY).toISOString()
const inMinutes = (minutes: number) => new Date(Date.now() + minutes * 60_000).toISOString()

/** Seulement pour effacer un état : la base jetable de web/test.sh. */
function sql(statement: string) {
  execFileSync('docker', ['exec', process.env.E2E_DB_CONTAINER!, 'psql', '-q', '-U', 'postgres', '-d', 'tracker', '-c', statement])
}

interface Status {
  state: 'ok' | 'action_required' | 'error'
  reason?: string | null
  expires_at?: string | null
}

async function report(request: APIRequestContext, token: string, status: Status) {
  const response = await request.put(`${API_URL}/v1/sources/me/status`, {
    headers: { Authorization: `Bearer ${token}` },
    data: status,
  })
  expect(response.status()).toBe(204)
}

test.afterEach(() => {
  sql("UPDATE core.sources SET status = NULL, status_reported_at = NULL WHERE name IN ('garmin', 'youtube')")
})

const sheet = (page: Page, id: string) => page.locator(`article[data-tracker="${id}"]`)
const expiryOf = (page: Page, id: string, key: RegExp) => sheet(page, id).getByRole('region', { name: key }).getByTestId('key-expiry')
const GARMIN_KEY = /^Clé d'API personnelle Garmin/
const GOOGLE_CONSENT = /^Accord Google/

async function openSources(page: Page) {
  await openMenu(page)
  await page.getByRole('link', { name: 'Sources' }).click()
  await expect(page.getByRole('heading', { level: 1, name: 'Sources' })).toBeVisible()
  // Sur iPhone, le tiroir a fini de se refermer.
  await expect(page.getByRole('dialog', { name: 'Menu' })).toBeHidden()
  expect(path(page.url())).toBe('/sources')
}

test('la page exige la session', async ({ page }) => {
  await page.goto('/sources')
  await expect(page.getByRole('heading', { name: 'Connexion' })).toBeVisible()
  await page.getByLabel('Identifiant').fill(USER)
  await page.getByLabel('Mot de passe').fill(PASSWORD)
  await page.getByRole('button', { name: 'Se connecter' }).click()
  await expect(page.getByRole('heading', { level: 1, name: 'Sources' })).toBeVisible()
  expect(path(page.url())).toBe('/sources')
})

test('les fiches, les autres clés et les échéances vivantes, sans aucune valeur secrète', async ({ page }) => {
  await report(page.request, GARMIN_TOKEN, { state: 'ok', expires_at: inDays(300) })
  await report(page.request, YOUTUBE_TOKEN, { state: 'ok', expires_at: inDays(5) })
  await login(page)
  await openSources(page)
  await openMenu(page)
  await expect(page.getByRole('link', { name: 'Sources' })).toHaveAttribute('aria-current', 'page')
  await page.keyboard.press('Escape')

  // Une fiche par tracker, dans l'ordre, puis les autres clés.
  const trackers = page.getByRole('region', { name: 'Trackers' }).getByRole('article')
  await expect(trackers.getByRole('heading', { level: 3 })).toHaveText(['Café', 'Musique', 'Garmin Connect', 'YouTube', 'Twitch', 'Steam', 'GitHub', 'Position (iPhone)'])
  const others = page.getByRole('region', { name: 'Autres clés à gérer' }).getByRole('article')
  await expect(others.getByRole('heading', { level: 3 })).toHaveText([
    'Claude (serveur MCP)',
    'Web app',
    'Base de données',
    'Images (ghcr.io)',
    'Déploiement automatique (secrets GitHub)',
    'Extension Firefox (AMO)',
  ])
  await expect(sheet(page, 'garmin')).toContainText("tous les quarts d'heure")
  await expect(sheet(page, 'twitch')).toContainText('segments de 60 s')
  await expect(sheet(page, 'steam')).toContainText("l'API Web de Steam tous les quarts d'heure")
  await expect(sheet(page, 'steam')).toContainText('STEAM_API_KEY')
  await expect(sheet(page, 'steam')).toContainText('Token de la source steam')
  await expect(sheet(page, 'iphone')).toContainText('Overland')
  await expect(sheet(page, 'iphone')).toContainText('Tailscale')
  await expect(sheet(page, 'iphone')).toContainText('Token de la source iphone')
  await expect(sheet(page, 'iphone').getByTestId('key-expiry')).toContainText('Sans échéance.')
  await expect(sheet(page, 'musique')).toContainText('Token de la source apple')
  for (const name of ['NAS_SSH_KEY', 'TS_OAUTH_SECRET', 'WEB_EXT_API_KEY', 'MCP_LOGIN_HASH', 'tracker_owner', 'read:packages']) {
    await expect(page.locator('main')).toContainText(name)
  }
  // L'état de chaque source, comme sur la page État.
  await expect(sheet(page, 'garmin').getByTestId('status-word')).toHaveText('Fonctionne')

  // Loin de l'échéance : la date, sans couleur. Le jeton Google à cinq jours : pas encore proche.
  await expect(expiryOf(page, 'garmin', GARMIN_KEY)).toContainText(/Valable jusqu'au \d+\s\S+\s\d{4}\./)
  await expect(expiryOf(page, 'garmin', GARMIN_KEY)).not.toHaveAttribute('data-warn')
  await expect(expiryOf(page, 'youtube', GOOGLE_CONSENT)).toContainText(/Valable jusqu'au/)
  await expect(expiryOf(page, 'youtube', GOOGLE_CONSENT)).not.toHaveAttribute('data-warn')
  // Une clé sans échéance au serveur garde son texte.
  await expect(sheet(page, 'cafe').getByTestId('key-expiry')).toContainText('Sans échéance.')

  // Aucun token, aucun mot de passe de l'environnement de test dans la page.
  const html = await page.content()
  for (const secret of [IOS_TOKEN, LASTFM_TOKEN, APPLE_TOKEN, GARMIN_TOKEN, YOUTUBE_TOKEN, TWITCH_TOKEN, STEAM_TOKEN, IPHONE_TOKEN, PASSWORD]) {
    expect(secret.length).toBeGreaterThan(8)
    expect(html.includes(secret)).toBe(false)
  }
  await checkLayout(page)

  // Proche : orange, et la fiche à faire comme la carte d'état.
  await report(page.request, GARMIN_TOKEN, { state: 'ok', expires_at: inDays(5) })
  await report(page.request, YOUTUBE_TOKEN, { state: 'ok', expires_at: inMinutes(120) })
  await page.reload()
  await expect(expiryOf(page, 'garmin', GARMIN_KEY)).toContainText(/Expire le \d+\s\S+, dans [45] jours\./)
  await expect(expiryOf(page, 'garmin', GARMIN_KEY)).toHaveAttribute('data-warn', 'soon')
  await expect(sheet(page, 'garmin').getByTestId('status-word')).toHaveText('À faire')
  await expect(expiryOf(page, 'youtube', GOOGLE_CONSENT)).toContainText(/Expire (à|demain à)/)
  await expect(expiryOf(page, 'youtube', GOOGLE_CONSENT)).toHaveAttribute('data-warn', 'soon')

  // Dépassée : rouge.
  await report(page.request, GARMIN_TOKEN, { state: 'action_required', reason: 'key_rejected', expires_at: inDays(-2) })
  await report(page.request, YOUTUBE_TOKEN, { state: 'action_required', reason: 'consent_expired', expires_at: null })
  await page.reload()
  await expect(expiryOf(page, 'garmin', GARMIN_KEY)).toContainText(/Expirée le \d+\s\S+/)
  await expect(expiryOf(page, 'garmin', GARMIN_KEY)).toHaveAttribute('data-warn', 'past')
  await expect(expiryOf(page, 'youtube', GOOGLE_CONSENT)).toHaveText(/Accord expiré\s: à redonner depuis la page État\./)
  await expect(expiryOf(page, 'youtube', GOOGLE_CONSENT)).toHaveAttribute('data-warn', 'past')
  const red = await expiryOf(page, 'garmin', GARMIN_KEY).locator('dd').evaluate((el) => getComputedStyle(el).color)
  const error = await page.evaluate(() => {
    const probe = document.createElement('span')
    probe.style.color = 'var(--state-error)'
    document.body.append(probe)
    const color = getComputedStyle(probe).color
    probe.remove()
    return color
  })
  expect(red).toBe(error)
})

for (const colorScheme of ['light', 'dark'] as const) {
  test.describe(colorScheme, () => {
    test.use({ colorScheme })

    /** Les captures : la clé Garmin proche, l'accord Google expiré. */
    async function scene(request: APIRequestContext) {
      await report(request, GARMIN_TOKEN, { state: 'ok', expires_at: inDays(5) })
      await report(request, YOUTUBE_TOKEN, { state: 'action_required', reason: 'consent_expired', expires_at: null })
    }

    test(`page Sources à 390 px, ${colorScheme}`, async ({ page, request }) => {
      await scene(request)
      await login(page)
      await openSources(page)
      await expect(expiryOf(page, 'garmin', GARMIN_KEY)).toHaveAttribute('data-warn', 'soon')
      await checkLayout(page)
      await page.screenshot({ path: `${SHOTS}/${colorScheme}-sources.png`, fullPage: true })
    })

    test.describe('écran large', () => {
      test.use({ viewport: DESKTOP, isMobile: false, hasTouch: false })

      test(`page Sources à 1280 px, ${colorScheme}`, async ({ page, request }) => {
        await scene(request)
        await login(page)
        await openSources(page)
        await expect(expiryOf(page, 'youtube', GOOGLE_CONSENT)).toHaveAttribute('data-warn', 'past')
        await checkLayout(page)
        await fullShot(page, `${SHOTS}/${colorScheme}-large-sources.png`)
      })
    })
  })
}
