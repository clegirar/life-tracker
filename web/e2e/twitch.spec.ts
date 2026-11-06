import { execFileSync } from 'node:child_process'
import { expect, test, type APIRequestContext, type Page } from '@playwright/test'
import { addDays, API_URL, apiSession, checkLayout, DESKTOP, fullShot, login, openMenu, parisToday, send, TWITCH_TOKEN } from './helpers'

// Twitch : la carte du jour et la carte de la page d'état. Les segments entrent par la vraie API
// d'ingestion avec le token de la source `twitch` (web/test.sh), comme l'extension Firefox ; son état
// aussi, par `PUT /v1/sources/me/status`. Seul le silence se simule dans la base jetable. Données fictives.
const SHOTS = process.env.E2E_SCREENSHOTS ?? 'e2e-artifacts/screenshots'
const HOUR = 3_600_000
const DATE = addDays(parisToday(), -13)
const EVE = addDays(DATE, -1)
const BEFORE = addDays(DATE, -20)

function sql(statement: string) {
  execFileSync('docker', ['exec', process.env.E2E_DB_CONTAINER!, 'psql', '-q', '-U', 'postgres', '-d', 'tracker', '-c', statement])
}

/** L'instant d'une heure de Paris, avec son décalage du jour (heure d'été ou d'hiver), comme l'extension. */
function paris(date: string, time: string): string {
  for (const offset of ['+01:00', '+02:00']) {
    const at = `${date}T${time}:00${offset}`
    const local = new Intl.DateTimeFormat('sv-SE', { timeZone: 'Europe/Paris', dateStyle: 'short', timeStyle: 'short' }).format(new Date(at))
    if (local === `${date} ${time}`) return at
  }
  throw new Error(`heure introuvable : ${date} ${time}`)
}

type Content = 'live' | 'vod' | 'clip'
const A = 'streamfictifa'
const B = 'streamfictifb'
const C = 'streamfictifc'
const nameOf = (login: string) => `StreamFictif${login.slice(-1).toUpperCase()}`

/** Un segment de lecture, une heure au plus. `null` : une chaîne que l'extension n'a pas su lire. */
const segment = (channel: string | null, content: Content, date: string, time: string, minutes: number) => ({
  key: `twitch-${channel ?? 'inconnue'}-${date}-${time}`,
  kind: 'twitch.watch',
  schema_version: 1,
  occurred_at: paris(date, time),
  payload: { channel, channel_name: channel && nameOf(channel), content, duration_ms: minutes * 60_000, category: channel ? 'Catégorie fictive' : null },
})

async function seed(request: APIRequestContext) {
  await send(
    request,
    [
      // Vingt jours avant : A n'est pas une découverte du jour.
      segment(A, 'live', BEFORE, '20:00', 10),
      // La veille, à cheval sur minuit : 30 min pour chaque jour.
      segment(C, 'vod', EVE, '23:30', 60),
      // A en direct deux heures ; B en même temps une demi-heure : l'union compte 120 min, pas 150.
      segment(A, 'live', DATE, '20:00', 60),
      segment(A, 'live', DATE, '21:00', 60),
      segment(B, 'clip', DATE, '21:00', 30),
      segment(null, 'live', DATE, '23:00', 10),
      segment(A, 'vod', DATE, '23:15', 15),
    ],
    TWITCH_TOKEN,
  )
  const cookie = await apiSession(request)
  await expect
    .poll(
      async () => {
        const body = await (await request.get(`/v1/days/${DATE}/stream`, { headers: { Cookie: cookie } })).json()
        return [body.minutes, body.summed_minutes, body.unknown_minutes]
      },
      { timeout: 30_000 },
    )
    .toEqual([175, 205, 10])
}

async function report(request: APIRequestContext, status: Record<string, unknown>) {
  const response = await request.put(`${API_URL}/v1/sources/me/status`, {
    headers: { Authorization: `Bearer ${TWITCH_TOKEN}` },
    data: status,
  })
  expect(response.status()).toBe(204)
}

async function heartbeat(request: APIRequestContext) {
  const response = await request.post(`${API_URL}/v1/heartbeat`, { headers: { Authorization: `Bearer ${TWITCH_TOKEN}` } })
  expect(response.status()).toBe(200)
}

const silence = () => sql("UPDATE core.sources SET last_seen_at = now() - interval '4 days' WHERE name = 'twitch'")
const card = (page: Page) => page.getByRole('article', { name: 'Twitch' })

/** Seulement Twitch dans les filtres, pour le mois et les captures : après la connexion, qui attend le café. */
async function onlyTwitch(page: Page) {
  await page.evaluate(() => window.localStorage.setItem('calendar.metrics', JSON.stringify(['stream.watch'])))
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

// Les autres fichiers retrouvent une source sans état ni silence.
test.afterEach(async ({ request }) => {
  sql("UPDATE core.sources SET status = NULL, status_reported_at = NULL WHERE name = 'twitch'")
  await heartbeat(request)
})

test('la carte du jour : union, temps par chaîne, minuit, chaîne inconnue et découvertes', async ({ page, request }) => {
  await heartbeat(request)
  await login(page)
  await page.goto(`/jour/${DATE}`)
  const twitch = page.getByTestId('twitch')
  await expect(twitch.getByRole('heading', { name: 'Twitch' })).toBeVisible()
  // 120 (A, B compris) + 10 (inconnue) + 15 (A) + 30 (C, après minuit) : l'union, 2 h 55.
  await expect(twitch.getByTestId('twitch-minutes')).toHaveText('2 h 55')
  await expect(twitch.getByTestId('twitch-overlap')).toHaveText('dont 30 min à deux streams')
  await expect(twitch.getByTestId('twitch-channel-count')).toHaveText('3')

  const rows = twitch.getByTestId('twitch-channel')
  await expect(rows).toHaveCount(4)
  // Par temps : A en entier malgré B, puis B et C à égalité, par nom ; l'inconnue à la fin.
  await expect(rows.nth(0)).toContainText('StreamFictifA')
  await expect(rows.nth(0)).toContainText(/Live · Rediffusion · 20:00 – 23:30/)
  await expect(rows.nth(0)).toContainText('2 h 15')
  await expect(rows.nth(1)).toContainText('StreamFictifB')
  await expect(rows.nth(1)).toContainText('Clip · 21:00 – 21:30')
  await expect(rows.nth(1)).toContainText('30 min')
  // Commencée la veille : sa demi-heure d'après minuit, et ses vraies heures.
  await expect(rows.nth(2)).toContainText('StreamFictifC')
  await expect(rows.nth(2)).toContainText('Rediffusion · 23:30 – 00:30')
  await expect(rows.nth(2)).toContainText('30 min')
  await expect(rows.nth(3)).toContainText('Chaîne inconnue')
  await expect(rows.nth(3)).toContainText('10 min')
  await expect(rows.nth(3).getByRole('link')).toHaveCount(0)

  const link = twitch.getByRole('link', { name: /StreamFictifA/ })
  await expect(link).toHaveAttribute('href', 'https://www.twitch.tv/streamfictifa')
  await expect(link).toHaveAttribute('target', '_blank')
  await expect(link).toHaveAttribute('rel', 'noopener noreferrer')
  await expect(twitch.getByRole('link')).toHaveCount(3)
  // Aucune image : ni avatar, ni miniature.
  await expect(twitch.locator('img')).toHaveCount(0)

  // Pas de « Nouvelles chaînes » : sans intérêt pour Twitch.
  await expect(twitch.getByTestId('twitch-new-channels')).toHaveCount(0)
  await checkLayout(page)

  // La veille : la demi-heure d'avant minuit.
  await page.goto(`/jour/${EVE}`)
  await expect(page.getByTestId('twitch-minutes')).toHaveText('30 min')
  await expect(page.getByTestId('twitch-overlap')).toHaveCount(0)

  // Avant le premier segment : pas de données, pas « pas de Twitch ».
  await page.goto(`/jour/${addDays(BEFORE, -1)}`)
  await expect(page.getByTestId('twitch')).toContainText('Pas de données Twitch à cette date.')
  // Entre deux soirées, Firefox ayant donné signe de vie depuis : pas de Twitch.
  await page.goto(`/jour/${addDays(DATE, 1)}`)
  await expect(page.getByTestId('twitch')).toContainText('Pas de Twitch ce jour-là.')
})

test('la case du mois : le temps du jour, en heures', async ({ page }) => {
  await login(page)
  await onlyTwitch(page)
  await page.goto(`/mois/${DATE.slice(0, 7)}`)
  const cell = page.locator(`.cell[data-date="${DATE}"]`)
  await expect(cell.locator('.cell-value[data-metric="stream.watch"]')).toHaveText('2,9 h')
  await expect(cell).toHaveAccessibleName(/2 h 55/)
  await expect(cell.locator('.cell-value')).toHaveClass(/heat-twitch/)
})

test("une source muette : l'alerte du jour, l'attente, et la carte d'état silencieuse", async ({ page, request }) => {
  silence()
  await login(page)
  await page.goto(`/jour/${DATE}`)
  await expect(page.getByTestId('twitch-alert')).toContainText('Firefox ne donne plus signe de vie.')
  // Après le dernier signal : on ne sait pas encore.
  await page.goto(`/jour/${parisToday()}`)
  await expect(page.getByTestId('twitch')).toContainText("Pas encore de données pour ce jour : Firefox n'a pas donné signe de vie depuis.")

  await openStatus(page)
  const twitch = card(page)
  await expect(twitch.getByTestId('status-word')).toHaveText('Silencieux')
  await expect(twitch).toContainText('Aucun signal de Firefox depuis plus de 3 jours : des streams peuvent manquer.')
  await expect(twitch).toContainText('about:addons')
  await expect(twitch).not.toContainText('collector')
  await checkLayout(page)

  // Le signal revient : la carte fonctionne, l'alerte disparaît.
  await heartbeat(request)
  await page.reload()
  await expect(twitch.getByTestId('status-word')).toHaveText('Fonctionne')
})

test("la page d'état : dernier signal, dernier envoi, dernier visionnage, puis l'envoi qui échoue", async ({ page, request }) => {
  await heartbeat(request)
  await report(request, { state: 'ok', last_collected_at: new Date(Date.now() - 3 * HOUR).toISOString() })
  await login(page)
  await openStatus(page)
  const twitch = card(page)
  await expect(twitch.getByTestId('status-word')).toHaveText('Fonctionne')
  await expect(twitch).toContainText("Dernier signal de Firefoxà l'instant")
  await expect(twitch).toContainText('Dernier envoiil y a 3 h')
  await expect(twitch).toContainText(/Dernier visionnage(le \d+\s\S+|hier) à/)
  await expect(twitch).not.toContainText('Dernière passe')

  await report(request, { state: 'error', reason: 'ingest_failed', last_collected_at: new Date(Date.now() - 3 * HOUR).toISOString() })
  await page.reload()
  await expect(twitch.getByTestId('status-word')).toHaveText('En panne')
  await expect(twitch).toContainText("L'extension n'arrive pas à remettre ses segments au serveur.")
  await expect(twitch).toContainText("Rien n'est perdu")
  await checkLayout(page)
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

        test(`Twitch à ${width} px, ${colorScheme}`, async ({ page, request }) => {
          const shot = (name: string) => fullShot(page, `${SHOTS}/${width}-${colorScheme}-${name}.png`)
          await heartbeat(request)
          await report(request, { state: 'ok', last_collected_at: new Date(Date.now() - 20 * 60_000).toISOString() })
          await login(page)
          await onlyTwitch(page)
          await page.goto(`/jour/${DATE}`)
          await expect(page.getByTestId('twitch-minutes')).toHaveText('2 h 55')
          await checkLayout(page)
          await shot('twitch-jour')

          await openStatus(page)
          await expect(card(page).getByTestId('status-word')).toHaveText('Fonctionne')
          await checkLayout(page)
          await shot('twitch-etat')

          silence()
          await page.reload()
          await expect(card(page).getByTestId('status-word')).toHaveText('Silencieux')
          await checkLayout(page)
          await shot('twitch-etat-silence')
        })
      })
    }
  })
}
