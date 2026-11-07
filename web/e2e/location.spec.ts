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
const STAYS = addDays(today, -32)
const AFTER = addDays(today, -30)
// Le séjour en cours se compte depuis maintenant : un instant fixé au chargement, pour qu'un renvoi des
// mêmes points retombe sur les mêmes clés.
const NOW = Math.floor(Date.now() / 60_000) * 60_000
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
// Les séjours détectés (un point par heure à l'arrêt), et le lieu où je suis encore aujourd'hui.
const MOORING: LatLng = [43.17, 5.2]
const BUOY: LatLng = [43.17, 5.23]
const HERE: LatLng = [43.235, 5.3]

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

/** Un point `minutes` avant `NOW`. */
const ago = (minutes: number, at: LatLng) => {
  const feature = point(DATE, '12:00', at)
  return { ...feature, properties: { ...feature.properties, timestamp: new Date(NOW - minutes * 60_000).toISOString().replace('.000Z', 'Z') } }
}

/**
 * Aujourd'hui : une marche de 20 min qui s'arrête à 330 m du lieu, puis un point en arrivant, il y a
 * 2 h, un autre une heure plus tard et un dernier il y a 12 min, comme Overland à l'arrêt. Envoyé
 * seulement après le test d'aujourd'hui sans position ; le lot porte toujours les mêmes clés.
 */
async function seedToday(request: APIRequestContext) {
  const start: LatLng = [HERE[0] - 0.003, HERE[1] - 0.025]
  const end: LatLng = [HERE[0] - 0.003, HERE[1]]
  await overland(request, [
    ...Array.from({ length: 21 }, (_, i) => ago(150 - i, [start[0], start[1] + ((end[1] - start[1]) * i) / 20])),
    ago(120, HERE),
    ago(60, [HERE[0] + 0.0002, HERE[1]]),
    ago(12, HERE),
  ])
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
    // Un point par heure à l'arrêt, au mouillage puis à la bouée, sans visite iOS au mouillage ; une
    // marche entre les deux, qui part à 330 m du mouillage. La visite iOS à la bouée recouvre le séjour
    // détecté là : un seul lieu, celui d'iOS.
    point(STAYS, '08:00', MOORING),
    point(STAYS, '09:00', [MOORING[0] + 0.0003, MOORING[1]]),
    point(STAYS, '10:00', MOORING),
    ...walk(STAYS, '10:05', [MOORING[0] + 0.003, MOORING[1]], [BUOY[0] + 0.003, BUOY[1]], 20),
    point(STAYS, '10:30', BUOY),
    point(STAYS, '11:30', BUOY),
    point(STAYS, '12:00', BUOY),
    visit([STAYS, '10:32'], [STAYS, '12:40'], BUOY),
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
        const stays = await (await request.get(`/v1/days/${STAYS}/locations`, { headers: { Cookie: cookie } })).json()
        return [body.covered, body.places.length, body.segments.length, body.point_count, stays.places.length]
      },
      { timeout: 30_000 },
    )
    .toEqual([true, 4, 3, 63, 2])
}

/** Le lieu en cours d'aujourd'hui, une fois les points interprétés. */
async function seedHere(request: APIRequestContext) {
  await seedToday(request)
  const cookie = await apiSession(request)
  await expect
    .poll(
      async () => {
        const body = await (await request.get(`/v1/days/${today}/locations`, { headers: { Cookie: cookie } })).json()
        return body.places.map((place: { origin: string; ongoing: boolean }) => [place.origin, place.ongoing])
      },
      { timeout: 30_000 },
    )
    .toEqual([['points', true]])
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

test('des séjours détectés dans les points, avec leur durée, une visite iOS à la place du séjour qu’elle recouvre', async ({ page }) => {
  await offlineTiles(page)
  await login(page)
  await page.goto(`/jour/${STAYS}`)
  const card = page.getByTestId('location')
  const rows = card.getByTestId('visit')
  // Le séjour au mouillage part au premier point de la marche ; à la bouée, la visite iOS seule.
  await expect(rows).toHaveCount(2)
  await expect(rows.nth(0)).toContainText('08:00 – 10:05')
  await expect(rows.nth(0)).toContainText('2 h 05')
  await expect(rows.nth(1)).toContainText('10:32 – 12:40')
  await expect(rows.nth(1)).toContainText('2 h 08')
  await expect(card.getByTestId('location-places')).toHaveText('2')
  await expect(card).not.toContainText('Aucun arrêt')
  await expect(card.getByTestId('location-here')).toHaveCount(0)
  await mapReady(page)
  await expect(card.locator('.visit-pin')).toHaveText(['1', '2'])
  await expect(card.locator('.visit-pin-current')).toHaveCount(0)
  // Le calendrier ne compte que la visite iOS : un séjour détecté n'est pas un événement.
  await onlyPlaces(page)
  await page.goto(`/mois/${STAYS.slice(0, 7)}`)
  await expect(page.locator(`.cell[data-date="${STAYS}"] .cell-value[data-metric="place.visit"]`)).toHaveText('1')
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

test('le lieu où je suis encore : « Ici depuis », le temps passé, et son épingle en évidence', async ({ page, request }) => {
  await seedHere(request)
  await offlineTiles(page)
  await login(page)
  await page.goto('/')
  const card = page.getByTestId('location')
  // Il y a 2 h : la durée se compte de minute d'horloge en minute d'horloge, la veille comprise après minuit.
  await expect(card.getByTestId('location-here').locator('.location-here-text')).toHaveText(/^Ici depuis .+ · 2 h 0[0-2]$/)
  const rows = card.getByTestId('visit')
  await expect(rows).toHaveCount(1)
  await expect(rows.nth(0)).toContainText(/depuis \d\d:\d\d/)
  await expect(rows.nth(0)).toContainText('en cours')
  await expect(rows.nth(0).locator('.visit-number-current')).toHaveCount(1)
  await mapReady(page)
  await expect(card.locator('.visit-pin-current')).toHaveText('1')
  await expect(card.locator('.route-map')).toHaveAccessibleName(/lieu 1 en cours$/)
  await checkLayout(page)
  // Un autre jour, rien n'est en cours.
  await page.goto(`/jour/${DATE}`)
  await expect(card.getByTestId('visit')).toHaveCount(4)
  await expect(card.getByTestId('location-here')).toHaveCount(0)
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

          await page.goto(`/jour/${STAYS}`)
          await mapReady(page)
          await page.evaluate(() => window.scrollTo(0, 0))
          await page.waitForTimeout(300)
          await checkLayout(page)
          await shot('deplacements-sejours')

          await seedHere(request)
          await page.goto('/')
          await expect(page.getByTestId('location-here')).toBeVisible()
          await mapReady(page)
          await page.evaluate(() => window.scrollTo(0, 0))
          await page.waitForTimeout(300)
          await checkLayout(page)
          await shot('deplacements-en-cours')

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
