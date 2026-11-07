import { execFileSync } from 'node:child_process'
import { expect, test, type APIRequestContext, type Page } from '@playwright/test'
import { addDays, API_URL, apiSession, checkLayout, DESKTOP, fullShot, IPHONE_TOKEN, login, openMenu, parisToday } from './helpers'

// Les déplacements (iPhone, Overland) : la carte du jour, la case du mois et la carte de la page d'état.
// Les lots entrent par la vraie route d'adaptation `POST /v1/locations/overland`, avec le token de la
// source `iphone` (web/test.sh), comme Overland ; Wi-Fi, batterie et identifiant d'appareil compris,
// pour vérifier qu'ils n'entrent pas. Seul le silence se simule dans la base jetable. Des coordonnées
// fictives, en mer au large de Marseille ; les tuiles d'OpenStreetMap sont une image locale.
const SHOTS = process.env.E2E_SCREENSHOTS ?? 'e2e-artifacts/screenshots'
const today = parisToday()
const BEFORE = addDays(today, -38)
const EVE = addDays(today, -37)
const DATE = addDays(today, -36)
const NEXT = addDays(today, -35)
const STILL = addDays(today, -34)
const AFTER = addDays(today, -30)
const TILE = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGN4+fQOAAVlAquKsz3oAAAAAElFTkSuQmCC', 'base64')

function sql(statement: string) {
  execFileSync('docker', ['exec', process.env.E2E_DB_CONTAINER!, 'psql', '-q', '-U', 'postgres', '-d', 'tracker', '-c', statement])
}

/** L'instant d'une heure de Paris, en UTC à la seconde, comme l'horodate Overland. */
function paris(date: string, time: string): string {
  for (const offset of ['+01:00', '+02:00']) {
    const at = new Date(`${date}T${time}:00${offset}`)
    const local = new Intl.DateTimeFormat('sv-SE', { timeZone: 'Europe/Paris', dateStyle: 'short', timeStyle: 'short' }).format(at)
    if (local === `${date} ${time}`) return at.toISOString().replace('.000Z', 'Z')
  }
  throw new Error(`heure introuvable : ${date} ${time}`)
}

/** « 08:45 » plus `minutes`. */
function later(time: string, minutes: number): string {
  const [h, m] = time.split(':').map(Number)
  const total = h * 60 + m + minutes
  return `${String(Math.floor(total / 60)).padStart(2, '0')}:${String(total % 60).padStart(2, '0')}`
}

type LatLng = [number, number]
const HOME: LatLng = [43.19, 5.24]
const CAFE: LatLng = [43.2055, 5.2705]
const LOOKOUT: LatLng = [43.2207, 5.2703]

const geometry = ([lat, lon]: LatLng) => ({ type: 'Point', coordinates: [lon, lat] })
// Ce qu'Overland ajoute et qui ne doit jamais entrer.
const noise = { device_id: 'iphone-fictif', wifi: 'Réseau fictif', wifi_bssid: '00:11:22:33:44:55', battery_level: 0.8, battery_state: 'unplugged' }

const point = (date: string, time: string, at: LatLng) => ({
  type: 'Feature',
  geometry: geometry(at),
  properties: { timestamp: paris(date, time), altitude: 2, speed: 1.4, course: 90, horizontal_accuracy: 8, vertical_accuracy: 4, motion: ['walking'], ...noise },
})

const visit = (arrival: [string, string], departure: [string, string] | null, at: LatLng) => ({
  type: 'Feature',
  geometry: geometry(at),
  properties: {
    action: 'visit',
    timestamp: paris(...(departure ?? arrival)),
    arrival_date: paris(...arrival),
    departure_date: departure && paris(...departure),
    horizontal_accuracy: 35,
    ...noise,
  },
})

/** Une marche d'une minute par point, de `from` vers `to`. */
function walk(date: string, start: string, from: LatLng, to: LatLng, steps: number) {
  return Array.from({ length: steps + 1 }, (_, i) =>
    point(date, later(start, i), [from[0] + ((to[0] - from[0]) * i) / steps, from[1] + ((to[1] - from[1]) * i) / steps]),
  )
}

async function overland(request: APIRequestContext, locations: unknown[]) {
  const response = await request.post(`${API_URL}/v1/locations/overland`, {
    headers: { Authorization: `Bearer ${IPHONE_TOKEN}` },
    data: { locations },
  })
  expect(response.status()).toBe(200)
  expect(await response.json()).toEqual({ result: 'ok' })
}

async function seed(request: APIRequestContext) {
  await overland(request, [
    // La nuit à la maison, depuis la veille.
    visit([EVE, '22:10'], [DATE, '08:40'], HOME),
    // Une marche vers le nord-est, un arrêt, une autre vers le nord, un arrêt.
    ...walk(DATE, '08:45', HOME, CAFE, 30),
    visit([DATE, '09:20'], [DATE, '11:05'], CAFE),
    ...walk(DATE, '11:10', CAFE, [43.2205, 5.2705], 30),
    visit([DATE, '11:45'], [DATE, '13:30'], LOOKOUT),
    // Un point isolé, puis la soirée et la nuit à la maison, jusqu'au lendemain.
    point(DATE, '15:00', [43.21, 5.255]),
    visit([DATE, '18:00'], [NEXT, '07:30'], HOME),
    // Une visite encore ouverte : iOS ne connaît pas le départ, elle n'entre pas.
    visit([NEXT, '20:00'], null, HOME),
    // Plus tard, un seul point : les jours d'entre-deux sont suivis, sans déplacement.
    point(AFTER, '12:00', HOME),
  ])
  // Un renvoi du même lot ne crée aucun doublon.
  await overland(request, [visit([DATE, '09:20'], [DATE, '11:05'], CAFE), point(DATE, '15:00', [43.21, 5.255])])
  const cookie = await apiSession(request)
  await expect
    .poll(
      async () => {
        const body = await (await request.get(`/v1/days/${DATE}/locations`, { headers: { Cookie: cookie } })).json()
        return [body.covered, body.visits.length, body.segments.length, body.point_count]
      },
      { timeout: 30_000 },
    )
    .toEqual([true, 4, 3, 63])
}

async function offlineTiles(page: Page) {
  await page.route('https://tile.openstreetmap.org/**', (route) => route.fulfill({ status: 200, contentType: 'image/png', body: TILE }))
}

/** Seulement les lieux dans les filtres : la carte Déplacements seule, pour le mois et les captures. */
async function onlyPlaces(page: Page) {
  await page.evaluate(() => window.localStorage.setItem('calendar.metrics', JSON.stringify(['place.visit'])))
}

async function openStatus(page: Page) {
  await openMenu(page)
  await page.getByRole('link', { name: /^État/ }).click()
  await expect(page.getByRole('heading', { level: 1, name: 'État' })).toBeVisible()
  await expect(page.getByRole('dialog', { name: 'Menu' })).toBeHidden()
}

const statusCard = (page: Page) => page.getByRole('article', { name: 'Position (iPhone)' })
const silence = () => sql("UPDATE core.sources SET last_seen_at = now() - interval '3 days' WHERE name = 'iphone'")

async function heartbeat(request: APIRequestContext) {
  const response = await request.post(`${API_URL}/v1/heartbeat`, { headers: { Authorization: `Bearer ${IPHONE_TOKEN}` } })
  expect(response.status()).toBe(200)
}

async function mapReady(page: Page) {
  const figure = page.getByTestId('location-map')
  await figure.scrollIntoViewIfNeeded()
  await expect(figure.locator('.route-map')).toHaveAttribute('data-state', 'ready')
}

test.beforeAll(async ({ request }) => {
  await seed(request)
})

// Les autres fichiers retrouvent une source qui parle.
test.afterEach(async ({ request }) => {
  await heartbeat(request)
})

test('la carte du jour : la trace, les lieux numérotés, la veille et le lendemain', async ({ page }) => {
  await offlineTiles(page)
  await login(page)
  await page.goto(`/jour/${DATE}`)
  const card = page.getByTestId('location')
  await expect(card.getByRole('heading', { name: 'Déplacements' })).toBeVisible()
  // Comme le calendrier, par arrivée : trois, et la nuit commencée la veille à part.
  await expect(card.getByTestId('location-places')).toHaveText('3')
  await expect(card).toContainText('et 1 depuis la veille')
  // Deux marches d'environ 3,0 et 1,7 km, sans relier les morceaux ni le point isolé.
  await expect(card.getByTestId('location-distance')).toHaveText(/^4,7\skm$/)

  const rows = card.getByTestId('visit')
  await expect(rows).toHaveCount(4)
  await expect(rows.nth(0)).toContainText('22:10 – 08:40')
  await expect(rows.nth(0)).toContainText('Depuis la veille')
  await expect(rows.nth(0)).toContainText('10 h 30')
  await expect(rows.nth(1)).toContainText('09:20 – 11:05')
  await expect(rows.nth(1)).toContainText('1 h 45')
  await expect(rows.nth(1).locator('.row-detail')).toHaveCount(0)
  await expect(rows.nth(2)).toContainText('11:45 – 13:30')
  await expect(rows.nth(3)).toContainText('18:00 – 07:30')
  await expect(rows.nth(3)).toContainText("Jusqu'au lendemain")
  await expect(rows.nth(3)).toContainText('13 h 30')
  // Aucun nom de lieu inventé : le numéro, lu « Lieu 1 » par un lecteur d'écran.
  await expect(rows.nth(0).getByRole('paragraph').first()).toHaveText('Lieu 1, 22:10 – 08:40')

  // La carte : deux morceaux de trace et le point isolé ; les deux nuits à la maison partagent une épingle.
  await mapReady(page)
  const map = card.locator('.route-map')
  await expect(map).toHaveAccessibleName('Carte des déplacements : 4 lieux, 4,7 km de trace')
  await expect(map.locator('.leaflet-overlay-pane path')).toHaveCount(3)
  await expect(map.locator('.visit-pin')).toHaveText(['1 · 4', '2', '3'])
  await expect(card.getByRole('link', { name: '© les contributeurs d’OpenStreetMap' })).toBeVisible()
  // Rien de ce qu'Overland ajoute : ni Wi-Fi ni appareil.
  await expect(card).not.toContainText('Réseau fictif')
  await checkLayout(page)

  // Le lendemain : la fin de la nuit, et la visite encore ouverte n'apparaît pas.
  await page.goto(`/jour/${NEXT}`)
  await expect(card.getByTestId('visit')).toHaveCount(1)
  await expect(card.getByTestId('visit')).toContainText('Depuis la veille')
  await expect(card.getByTestId('location-places')).toHaveText('0')
  await expect(card.getByTestId('location-distance')).toHaveText('–')
  await expect(card).toContainText('Aucune trace')
})

test('un jour suivi sans déplacement, un jour sans données, aujourd’hui sans position', async ({ page }) => {
  await login(page)
  await page.goto(`/jour/${STILL}`)
  const card = page.getByTestId('location')
  await expect(card.getByTestId('location-empty')).toHaveText('Aucun déplacement enregistré ce jour-là.')
  await expect(card.getByTestId('location-map')).toHaveCount(0)
  // Avant la première donnée : pas de données, jamais « aucun déplacement ».
  await page.goto(`/jour/${BEFORE}`)
  await expect(card.getByTestId('location-empty')).toHaveText('Pas de données de position à cette date.')
  await page.goto('/')
  await expect(card.getByTestId('location-empty')).toHaveText("Pas encore de position reçue aujourd'hui.")
})

test('la case du mois : le nombre de lieux, par arrivée, dans sa couleur', async ({ page }) => {
  await login(page)
  await onlyPlaces(page)
  await page.goto(`/mois/${DATE.slice(0, 7)}`)
  const cell = page.locator(`.cell[data-date="${DATE}"]`)
  await expect(cell.locator('.cell-value[data-metric="place.visit"]')).toHaveText('3')
  await expect(cell.locator('.cell-value')).toHaveClass(/heat-location/)
  await expect(cell).toHaveAccessibleName(/3 lieux/)
  await expect(page.getByRole('group', { name: 'Métriques affichées' }).getByRole('button', { name: 'Déplacements' })).toHaveClass(/heat-location/)
})

test("la page d'état : dernier envoi, dernière position et dernier lieu, puis le silence", async ({ page, request }) => {
  await heartbeat(request)
  await login(page)
  await openStatus(page)
  const card = statusCard(page)
  await expect(card.getByTestId('status-word')).toHaveText('Fonctionne')
  await expect(card).toContainText("Dernier envoi de l'iPhoneà l'instant")
  await expect(card).toContainText(/Dernière position(le \d+\s\S+|hier) à/)
  await expect(card).toContainText(/Dernier lieu(le \d+\s\S+|hier) à/)
  await expect(card).not.toContainText('Dernière passe')

  silence()
  await page.reload()
  await expect(card.getByTestId('status-word')).toHaveText('Silencieux')
  await expect(card).toContainText("Aucun envoi de l'iPhone depuis plus de 2 jours : des déplacements peuvent manquer.")
  await expect(card).toContainText('Tailscale')
  await expect(card).toContainText('« Toujours »')
  await expect(card).not.toContainText('collector')
  await checkLayout(page)
  // Et l'alerte de la carte du jour.
  await page.goto(`/jour/${DATE}`)
  await expect(page.getByTestId('location-alert')).toContainText("L'iPhone n'envoie plus sa position.")
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

        test(`Déplacements à ${width} px, ${colorScheme}`, async ({ page, request }) => {
          const shot = (name: string) => fullShot(page, `${SHOTS}/${width}-${colorScheme}-${name}.png`)
          await heartbeat(request)
          await offlineTiles(page)
          await login(page)
          await onlyPlaces(page)
          await page.goto(`/jour/${DATE}`)
          await mapReady(page)
          await page.evaluate(() => window.scrollTo(0, 0))
          await page.waitForTimeout(300) // la fin du fondu de la carte
          await checkLayout(page)
          await shot('deplacements-jour')

          await page.goto(`/jour/${STILL}`)
          await expect(page.getByTestId('location-empty')).toBeVisible()
          await checkLayout(page)
          await shot('deplacements-jour-immobile')

          await openStatus(page)
          await expect(statusCard(page).getByTestId('status-word')).toHaveText('Fonctionne')
          await checkLayout(page)
          await shot('deplacements-etat')

          silence()
          await page.reload()
          await expect(statusCard(page).getByTestId('status-word')).toHaveText('Silencieux')
          await checkLayout(page)
          await shot('deplacements-etat-silence')
        })
      })
    }
  })
}
