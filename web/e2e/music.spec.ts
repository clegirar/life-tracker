import { expect, test, type APIRequestContext, type Page } from '@playwright/test'
import { addDays, API_URL, apiSession, LASTFM_TOKEN, login, parisToday } from './helpers'

// Les écoutes entrent par la vraie API d'ingestion, avec le token de la source, comme le collecteur.
// Noms fictifs. Chaque test a sa journée : le journal n'efface rien.
const today = parisToday()
const SHOTS = process.env.E2E_SCREENSHOTS ?? 'e2e-artifacts/screenshots'
const music = (page: Page) => page.getByTestId('music')
const noon = (date: string, minute = 0) => `${date}T12:${String(minute).padStart(2, '0')}:00+01:00`

interface Listen {
  artist: string
  track: string
  album?: string
}

async function send(request: APIRequestContext, records: unknown[]) {
  const response = await request.post(`${API_URL}/v1/records`, {
    headers: { Authorization: `Bearer ${LASTFM_TOKEN}` },
    data: { records },
  })
  expect(response.status()).toBe(200)
  const body = await response.json()
  for (const result of body.results) expect(['accepted', 'duplicate']).toContain(result.status)
}

const scrobble = (date: string, minute: number, { artist, track, album = '' }: Listen) => ({
  key: `${date}-${minute}-${artist}-${track}`,
  kind: 'lastfm.scrobble',
  schema_version: 1,
  occurred_at: noon(date, minute),
  payload: {
    artist: { mbid: '', '#text': artist },
    album: { mbid: '', '#text': album },
    name: track,
    date: { uts: '0', '#text': '' },
  },
})

const info = (artist: string, track: string, ms: string) => ({
  key: `info-${artist}-${track}`,
  kind: 'lastfm.track_info',
  schema_version: 1,
  occurred_at: noon(addDays(today, -40)),
  payload: { artist, track, info: { track: { name: track, duration: ms } } },
})

/** Attend que le worker ait tout interprété, sans supposer de sa vitesse. */
async function waitForListens(request: APIRequestContext, cookie: string, date: string, expected: number) {
  await expect
    .poll(
      async () => {
        const response = await request.get(`/v1/days/${date}/music`, { headers: { Cookie: cookie } })
        return (await response.json()).listens
      },
      { timeout: 30_000 },
    )
    .toBe(expected)
}

async function previousDay(page: Page, times: number) {
  for (let i = 0; i < times; i++) await page.getByRole('button', { name: 'Jour précédent' }).click()
}

const DAY: Listen[] = [
  { artist: 'Artiste Fictif', track: 'Titre fictif', album: 'Album fictif' },
  { artist: 'Artiste Fictif', track: 'Titre fictif - Remastered 2011', album: 'Album fictif' },
  { artist: 'ARTISTE FICTIF feat. Invité', track: 'Autre titre (Live)', album: 'Album fictif' },
  { artist: 'Second Groupe', track: 'Sans durée connue', album: 'Autre album' },
  { artist: 'Troisième', track: 'Sans album' },
]

async function seedDay(request: APIRequestContext, cookie: string, date: string) {
  await send(request, [
    ...DAY.map((listen, minute) => scrobble(date, minute, listen)),
    info('Artiste Fictif', 'Titre fictif', '180000'),
    info('Artiste Fictif', 'Autre titre', '300000'),
    info('Second Groupe', 'Sans durée connue', '0'),
  ])
  await waitForListens(request, cookie, date, DAY.length)
}

test('la section musique montre le nombre, le temps, l\'artiste du jour et les trois listes', async ({ page, request }) => {
  const date = addDays(today, -2)
  const cookie = await apiSession(request)
  await seedDay(request, cookie, date)

  await login(page)
  await previousDay(page, 2)
  await expect(page.getByTestId('listens')).toHaveText('5')
  // 180 × 2 + 300 = 660 s ; deux écoutes sont sans durée connue.
  await expect(page.getByTestId('minutes')).toHaveText('11 min')
  await expect(music(page).getByText('Sans compter 2 écoutes de durée inconnue.')).toBeVisible()
  await expect(music(page).getByText('ARTISTE FICTIF feat. Invité · 3 écoutes')).toBeVisible()

  const rows = music(page).locator('.row')
  // Deux versions d'un titre comptent pour un seul.
  await expect(rows).toHaveCount(4)
  await expect(rows.first()).toContainText('Titre fictif - Remastered 2011')
  await expect(rows.first().locator('.row-count')).toHaveText('2')

  await music(page).getByRole('button', { name: 'Artistes' }).click()
  await expect(music(page).getByRole('button', { name: 'Artistes' })).toHaveAttribute('aria-pressed', 'true')
  await expect(rows).toHaveCount(3)
  await expect(rows.first()).toContainText('2 titres')
  await expect(rows.first().locator('.row-count')).toHaveText('3')

  await music(page).getByRole('button', { name: 'Albums' }).click()
  await expect(rows).toHaveCount(2)
  await expect(rows.first()).toContainText('Album fictif')
  await expect(music(page).getByText('Sans album')).toHaveCount(0)
})

test('la journée de Paris : une écoute à 00:30 est du jour, pas de la veille', async ({ page, request }) => {
  const date = addDays(today, -3)
  const cookie = await apiSession(request)
  // 23:30 UTC la veille en hiver : 00:30 à Paris.
  await send(request, [
    {
      ...scrobble(date, 0, { artist: 'Nuit Fictive', track: 'Minuit passé' }),
      occurred_at: `${date}T00:30:00+01:00`,
    },
  ])
  await waitForListens(request, cookie, date, 1)
  await login(page)
  await previousDay(page, 3)
  await expect(music(page).getByText('Minuit passé')).toBeVisible()
  await previousDay(page, 1)
  await expect(music(page).getByText('Minuit passé')).toHaveCount(0)
})

test('une journée sans écoute et une panne ne se confondent pas', async ({ page, request }) => {
  const cookie = await apiSession(request)
  await seedDay(request, cookie, addDays(today, -2))
  await login(page)
  await previousDay(page, 1)
  await expect(music(page).getByText('Aucune écoute hier.')).toBeVisible()
  await expect(page.getByTestId('listens')).toHaveCount(0)

  // Le connecteur se tait : la même journée n'est plus une journée sans écoute.
  await page.route('**/v1/sources/health', async (route) => {
    const response = await route.fetch()
    const health = await response.json()
    health.sources[0].silent = true
    await route.fulfill({ response, json: health })
  })
  await page.reload()
  await previousDay(page, 1)
  await expect(page.getByRole('heading', { name: 'Hier' })).toBeVisible()
  await expect(page.getByTestId('music-alert')).toContainText('Last.fm ne répond plus.')
  await expect(music(page)).toContainText('Pas de données')
  await expect(music(page).getByText('Aucune écoute hier.')).toHaveCount(0)
})

test('une panne de lecture ne ressemble pas à zéro écoute', async ({ page }) => {
  await page.route('**/v1/days/*/music', (route) => route.fulfill({ status: 500, body: '{}' }))
  await login(page)
  await expect(music(page).getByText("Impossible d'actualiser la musique.")).toBeVisible()
  await expect(music(page).getByText(/Aucune écoute/)).toHaveCount(0)
  await page.unroute('**/v1/days/*/music')
  await music(page).getByRole('button', { name: 'Réessayer' }).click()
  await expect(music(page).getByText("Impossible d'actualiser la musique.")).toHaveCount(0)
})

async function checkLayout(page: Page) {
  const problems = await page.evaluate(() => {
    const out: string[] = []
    if (document.documentElement.scrollWidth > window.innerWidth) out.push('défilement horizontal')
    for (const el of document.querySelectorAll('button, input, a')) {
      const { width, height, right } = el.getBoundingClientRect()
      const name = el.getAttribute('aria-label') ?? el.textContent ?? el.tagName
      if (height < 44 || width < 44) out.push(`${name} : ${width}×${height}`)
      if (right > window.innerWidth) out.push(`${name} dépasse à droite`)
    }
    return out
  })
  expect(problems).toEqual([])
}

for (const [colorScheme, daysAgo] of [['light', 4], ['dark', 5]] as const) {
  test.describe(colorScheme, () => {
    test.use({ colorScheme })

    test(`musique à 390 px, ${colorScheme}`, async ({ page, request }) => {
      const date = addDays(today, -daysAgo)
      const cookie = await apiSession(request)
      await seedDay(request, cookie, date)
      const long = 'Un Nom Fictif Extrêmement Long Qui Ne Tient Pas Sur Une Seule Ligne De Téléphone'
      await send(request, [scrobble(date, 30, { artist: `${long}Sansespacedutoutpourvoirsilepagedéborde`, track: long, album: long })])
      await waitForListens(request, cookie, date, DAY.length + 1)
      const shot = (name: string) => page.screenshot({ path: `${SHOTS}/${colorScheme}-musique-${name}.png`, fullPage: true })

      await login(page)
      await previousDay(page, daysAgo)
      await expect(page.getByTestId('listens')).toHaveText(String(DAY.length + 1))
      expect(page.viewportSize()!.width).toBe(390)
      await checkLayout(page)
      await shot('1-titres')
      await music(page).getByRole('button', { name: 'Artistes' }).click()
      await checkLayout(page)
      await shot('2-artistes')
      await music(page).getByRole('button', { name: 'Albums' }).click()
      await checkLayout(page)
      await shot('3-albums')

      await page.route('**/v1/sources/health', async (route) => {
        const response = await route.fetch()
        const health = await response.json()
        health.metrics[0].stale = true
        await route.fulfill({ response, json: health })
      })
      await page.reload()
      await expect(page.getByTestId('music-alert')).toContainText('Musique à vérifier.')
      await checkLayout(page)
      await shot('4-a-verifier')
    })
  })
}
