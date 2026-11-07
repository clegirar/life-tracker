import { expect, test, type APIRequestContext, type Page } from '@playwright/test'
import { addDays, apiSession, checkLayout, DESKTOP, fullShot, GARMIN_TOKEN, login, parisToday, send } from './helpers'

// La carte Activité (Garmin) et le calendrier à six métriques. Des jours que les autres fichiers
// n'utilisent pas : 8 et 9 jours avant aujourd'hui. Données, lieu et tracé fictifs ; les tuiles
// d'OpenStreetMap sont remplacées par une image locale : aucun appel réseau.
const SHOTS = process.env.E2E_SCREENSHOTS ?? 'e2e-artifacts/screenshots'
const today = parisToday()
const DAY = addDays(today, -8)
const NO_DATA = addDays(today, -9)
const PLACE = 'Ville Fictive'
const TILE = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGN4+fQOAAVlAquKsz3oAAAAAElFTkSuQmCC', 'base64')

/** Midi à Paris, avec son décalage, comme le connecteur date un total du jour. */
function parisNoon(date: string): string {
  const offset = new Intl.DateTimeFormat('en', { timeZone: 'Europe/Paris', timeZoneName: 'longOffset' })
    .formatToParts(new Date(`${date}T12:00:00Z`))
    .find((part) => part.type === 'timeZoneName')!
    .value.replace('GMT', '')
  return `${date}T12:00:00${offset || '+00:00'}`
}

/**
 * Un parcours fictif : les sept huitièmes d'un ovale d'environ 1,6 km sur 0,9, départ et arrivée à plus
 * de 30 m l'un de l'autre, une allure qui varie d'un tronçon à l'autre (de 13 à 31 s entre deux points).
 */
function loop(): [number, number, number][] {
  let seconds = 0
  return Array.from({ length: 80 }, (_, i) => {
    const angle = (i / 79) * 1.75 * Math.PI
    if (i > 0) seconds += Math.round(22 + 9 * Math.sin(3 * angle))
    return [
      Math.round((48.85 + 0.004 * Math.sin(angle)) * 1e6) / 1e6,
      Math.round((2.35 + 0.011 * (1 - Math.cos(angle))) * 1e6) / 1e6,
      seconds,
    ]
  })
}

const garmin = (key: string, kind: string, occurredAt: string, payload: Record<string, unknown>) => ({
  key,
  kind,
  schema_version: 1,
  occurred_at: occurredAt,
  payload,
})

async function seed(request: APIRequestContext, cookie: string) {
  const at = parisNoon(DAY)
  const runStart = `${DAY}T06:30:00Z`
  const points = loop()
  await send(
    request,
    [
      garmin(`summary:${DAY}:e2e`, 'garmin.daily_summary', at, {
        calendar_date: DAY, steps: 8412, distance_m: 6123, calories_total: 2350, calories_active: 512, calories_resting: 1838,
      }),
      garmin(`sleep:${DAY}:e2e`, 'garmin.sleep', at, {
        calendar_date: DAY, start: `${addDays(DAY, -1)}T21:41:00Z`, end: `${DAY}T05:05:00Z`, sleep_seconds: 25_980,
        deep_seconds: 4_920, light_seconds: 14_400, rem_seconds: 6_000, awake_seconds: 660,
        // Une nuit de 7 h 24, un échantillon toutes les deux minutes : 48 à 55 bpm, fictifs.
        heart_rate: Array.from({ length: 223 }, (_, i) => [i * 120, 48 + Math.round(7 * Math.abs(Math.sin(i / 20)))]),
      }),
      garmin('activity:9001:e2e', 'garmin.activity', runStart, {
        activity_id: 9001, type: 'running', start: runStart, duration_s: 1802, moving_s: 1780, distance_m: 5012.3,
        avg_speed_mps: 2.78, avg_hr: 151, max_hr: 176, calories: 389, start_lat: points[0][0], start_lon: points[0][1],
        end_lat: points[79][0], end_lon: points[79][1], location_name: PLACE,
      }),
      garmin('track:9001:e2e', 'garmin.activity_track', runStart, { activity_id: 9001, points }),
      garmin('activity:9002:e2e', 'garmin.activity', `${DAY}T16:10:00Z`, {
        activity_id: 9002, type: 'walking', start: `${DAY}T16:10:00Z`, duration_s: 1500, moving_s: 1440, distance_m: 1800,
        avg_speed_mps: 1.25, avg_hr: 98, max_hr: 112, calories: 120, start_lat: null, start_lon: null, end_lat: null,
        end_lon: null, location_name: null,
      }),
    ],
    GARMIN_TOKEN,
  )
  await expect
    .poll(async () => (await (await request.get(`/v1/days/${DAY}/activity`, { headers: { Cookie: cookie } })).json()).workouts.length, {
      timeout: 15_000,
    })
    .toBe(2)
}

/** Les tuiles d'OpenStreetMap, servies par une image locale : aucun appel au vrai serveur. */
async function offlineTiles(page: Page) {
  await page.route('https://tile.openstreetmap.org/**', (route) => route.fulfill({ status: 200, contentType: 'image/png', body: TILE }))
}

async function openDay(page: Page, date: string) {
  await login(page)
  await page.goto(`/jour/${date}`)
  await expect(page.getByTestId('activity')).toBeVisible()
}

test('la carte Activité : pas, calories, sommeil et séances, avec le parcours', async ({ page, request }) => {
  await seed(request, await apiSession(request))
  await offlineTiles(page)
  await openDay(page, DAY)
  const card = page.getByTestId('activity')
  await expect(card.getByTestId('steps')).toHaveText(/^8\s412$/)
  await expect(card.getByTestId('calories')).toHaveText(/^2\s350 kcal$/)
  await expect(card).toContainText(/512 actives · 1\s838 au repos/)
  await expect(card.getByTestId('sleep')).toHaveText('7 h 13')
  await expect(card).toContainText('22:41 – 06:05')
  await expect(card.getByTestId('sleep-heart-rate')).toHaveText('FC la plus basse 48 bpm · moyenne 53 bpm')
  await expect(card).toContainText('Paradoxal')
  const workouts = card.getByTestId('workout')
  await expect(workouts).toHaveCount(2)
  await expect(workouts.nth(0)).toContainText('Course')
  await expect(workouts.nth(0)).toContainText(`07:30 · ${PLACE}`)
  await expect(workouts.nth(0)).toContainText('5,01 km')
  await expect(workouts.nth(0)).toContainText('6:00 /km')
  await expect(workouts.nth(0)).toContainText('FC moyenne151 bpm')
  await expect(workouts.nth(0)).toContainText('FC max176 bpm')
  await expect(workouts.nth(1)).toContainText('Marche')
  await expect(workouts.nth(1)).toContainText('4,5 km/h')
  // Une carte pour la séance avec GPS seulement, tracé, départ et arrivée.
  await expect(card.getByTestId('route-map')).toHaveCount(1)
  // La carte ne se monte qu'à l'approche : les filtres du jour la poussent sous l'écran d'un iPhone.
  await card.getByTestId('route-map').scrollIntoViewIfNeeded()
  const map = card.locator('.route-map')
  await expect(map).toHaveAttribute('data-state', 'ready')
  await expect(map).toHaveAccessibleName('Parcours : course de 07:30')
  // Un liseré sous la trace, puis un morceau par suite de tronçons de même allure.
  expect(await map.locator('.leaflet-overlay-pane path').count()).toBeGreaterThan(3)
  // Départ et arrivée, deux repères distincts par leur forme et leur nom.
  await expect(map.locator('.route-marker [role="img"]')).toHaveCount(2)
  await expect(map.locator('.route-marker [aria-label="Départ"] circle')).toHaveCount(1)
  await expect(map.locator('.route-marker [aria-label="Arrivée"] rect')).toHaveCount(1)
  // La légende : les bornes de l'échelle, en allure de course, la plus lente d'abord.
  const legend = card.getByTestId('pace-legend')
  await expect(legend).toHaveText(/^Couleur de la trace selon l’allure : Lent \d+:\d\d \/kmRapide \d+:\d\d \/km$/)
  const [slow, fast] = (await legend.locator('strong').allTextContents()).map((text) => {
    const [m, s] = text.split(' ')[0].split(':').map(Number)
    return m * 60 + s
  })
  expect(slow).toBeGreaterThan(fast)
  await expect(card.getByRole('link', { name: '© les contributeurs d’OpenStreetMap' })).toBeVisible()
  await checkLayout(page)
})

/** Le zoom de la carte, lu dans l'adresse de ses tuiles : `null` tant que deux zooms se mêlent. */
const zoomLevel = (page: Page) =>
  page.locator('.route-map img.leaflet-tile').evaluateAll((tiles) => {
    const levels = new Set(tiles.map((tile) => Number(new URL((tile as HTMLImageElement).src).pathname.split('/')[1])))
    return levels.size === 1 ? [...levels][0] : null
  })

test('les boutons de zoom : zoomer, dézoomer, recentrer, et le glisser au toucher une fois zoomé', async ({ page, request }) => {
  await seed(request, await apiSession(request))
  await offlineTiles(page)
  await openDay(page, DAY)
  const figure = page.getByTestId('route-map')
  await figure.scrollIntoViewIfNeeded()
  const map = figure.locator('.route-map')
  await expect(map).toHaveAttribute('data-state', 'ready')
  const zoomIn = figure.getByRole('button', { name: 'Zoomer', exact: true })
  const zoomOut = figure.getByRole('button', { name: 'Dézoomer' })
  const recenter = figure.getByRole('button', { name: 'Recentrer' })
  await expect(zoomIn).toBeEnabled()
  await expect(zoomOut).toBeEnabled()
  // La vue d'origine : rien à recentrer, et au toucher, un doigt fait défiler la page.
  await expect(recenter).toHaveCount(0)
  const touch = await page.evaluate(() => window.matchMedia('(pointer: coarse)').matches)
  expect(touch).toBe(true)
  await expect(map).not.toHaveClass(/leaflet-touch-drag/)
  const home = await zoomLevel(page)
  expect(home).not.toBeNull()

  await zoomIn.click()
  await expect.poll(() => zoomLevel(page)).toBe(home! + 1)
  await expect(recenter).toBeVisible()
  // Zoomée : le doigt déplace la carte.
  await expect(map).toHaveClass(/leaflet-touch-drag/)
  await checkLayout(page)

  await recenter.click()
  await expect(recenter).toHaveCount(0)
  await expect.poll(() => zoomLevel(page)).toBe(home)
  await expect(map).not.toHaveClass(/leaflet-touch-drag/)

  // Dézoomer : la vue change aussi, mais le doigt fait toujours défiler la page.
  await zoomOut.click()
  await expect.poll(() => zoomLevel(page)).toBe(home! - 1)
  await expect(recenter).toBeVisible()
  await expect(map).not.toHaveClass(/leaflet-touch-drag/)

  // Au plus près (19), « Zoomer » se grise.
  for (let zoom = home! - 1; zoom < 19; zoom++) {
    await zoomIn.click()
    await expect.poll(() => zoomLevel(page)).toBe(zoom + 1)
  }
  await expect(zoomIn).toBeDisabled()
  await expect(zoomOut).toBeEnabled()
})

/**
 * Les cartes du jour, colonne par colonne (par leur bord gauche), et l'écart vertical entre deux cartes
 * qui se suivent dans une colonne.
 */
const columnGaps = (page: Page) =>
  page.locator('.cards > *').evaluateAll((cards) => {
    const columns = new Map<number, DOMRect[]>()
    for (const card of cards) {
      const rect = card.getBoundingClientRect()
      const left = Math.round(rect.left)
      columns.set(left, [...(columns.get(left) ?? []), rect])
    }
    return [...columns.values()].map((rects) =>
      rects.sort((a, b) => a.top - b.top).slice(1).map((rect, i) => rect.top - rects[i].bottom),
    )
  })

test('la page du jour en maçonnerie : deux colonnes sur Mac, sans trou entre les cartes', async ({ page, request }) => {
  await seed(request, await apiSession(request))
  await offlineTiles(page)
  await page.setViewportSize(DESKTOP)
  await openDay(page, DAY)
  await page.getByTestId('route-map').scrollIntoViewIfNeeded()
  await expect(page.locator('.route-map')).toHaveAttribute('data-state', 'ready')
  await expect(page.locator('.cards')).toHaveAttribute('data-masonry', '')
  // L'ordre du DOM reste celui de la lecture.
  await expect(page.locator('.cards > .card').first()).toHaveClass(/card-coffee/)
  const gap = await page.locator('.cards').evaluate((el) => parseFloat(getComputedStyle(el).columnGap))
  expect(gap).toBe(20)
  const check = async () => {
    const columns = await columnGaps(page)
    expect(columns).toHaveLength(2)
    for (const gaps of columns) {
      for (const between of gaps) {
        expect(between).toBeGreaterThanOrEqual(gap - 0.5)
        expect(between).toBeLessThan(gap + 1)
      }
    }
  }
  await check()
  // Une carte en moins : les autres se replacent, toujours sans trou.
  await page.getByRole('group', { name: 'Métriques affichées' }).getByRole('button', { name: 'Écoutes' }).click()
  await expect(page.getByTestId('music')).toHaveCount(0)
  await check()

  // Sur iPhone, une seule colonne, au même écart.
  await page.setViewportSize({ width: 390, height: 844 })
  await expect.poll(async () => (await columnGaps(page)).length).toBe(1)
  for (const between of (await columnGaps(page))[0]) {
    expect(between).toBeGreaterThanOrEqual(gap - 0.5)
    expect(between).toBeLessThan(gap + 1)
  }
})

test('un jour sans données Garmin le dit, sans zéro', async ({ page }) => {
  await openDay(page, NO_DATA)
  const card = page.getByTestId('activity')
  await expect(card).toContainText('Pas de données Garmin ce jour-là.')
  await expect(card.getByTestId('steps')).toHaveCount(0)
})

test('sept pastilles : toutes choisies par défaut, et « Activité » coche les quatre métriques Garmin ensemble', async ({ page, request }) => {
  await seed(request, await apiSession(request))
  await login(page)
  await page.goto(`/mois/${DAY.slice(0, 7)}`)
  const filters = page.getByRole('group', { name: 'Métriques affichées' })
  // Dans l'ordre d'affichage du registre ; les quatre métriques Garmin sous une pastille, les succès sans pastille.
  await expect(filters.locator('.chip')).toHaveText(['Cafés', 'Écoutes', 'Activité', 'Vidéos YouTube', 'Twitch', 'Jeux', 'Déplacements'])
  await expect(filters.getByRole('button', { name: 'Activité' })).toHaveClass(/heat-steps/)
  for (const name of ['Cafés', 'Écoutes', 'Activité', 'Vidéos YouTube', 'Twitch', 'Jeux', 'Déplacements']) {
    await expect(filters.getByRole('button', { name })).toHaveAttribute('aria-pressed', 'true')
  }
  const cell = page.locator(`.cell[data-date="${DAY}"]`)
  const garmin = ['steps', 'calories.burned', 'sleep', 'workout']
  for (const metric of garmin) {
    await expect(cell.locator(`.day-dot[data-metric="${metric}"]`)).toHaveCount(1)
  }
  await expect(cell).toHaveAccessibleName(/8\s412 pas.*2 séances.*7 h 13.*2\s350 kcal/)
  // Activité seule : ses quatre points, sans les autres.
  for (const name of ['Cafés', 'Écoutes', 'Vidéos YouTube', 'Twitch', 'Jeux', 'Déplacements']) await filters.getByRole('button', { name }).click()
  for (const metric of garmin) {
    await expect(cell.locator(`.day-dot[data-metric="${metric}"]`)).toHaveCount(1)
  }
  await expect(cell.locator('.day-dot')).toHaveCount(4)
  // Décochée : les quatre partent ensemble.
  await filters.getByRole('button', { name: 'Activité' }).click()
  await expect(filters.getByRole('button', { name: 'Activité' })).toHaveAttribute('aria-pressed', 'false')
  await expect(cell.locator('.day-dot')).toHaveCount(0)
  await expect(page.getByTestId('no-metric')).toBeVisible()
})

for (const colorScheme of ['light', 'dark'] as const) {
  test.describe(colorScheme, () => {
    test.use({ colorScheme })

    for (const [width, viewport] of [['390', { width: 390, height: 844 }], ['1280', DESKTOP]] as const) {
      test(`Activité, mois et année à ${width} px, ${colorScheme}`, async ({ page, request }) => {
        await seed(request, await apiSession(request))
        await offlineTiles(page)
        await page.setViewportSize(viewport)
        const shot = (name: string) => fullShot(page, `${SHOTS}/${width}-${colorScheme}-${name}.png`)
        await openDay(page, DAY)
        await page.getByTestId('route-map').scrollIntoViewIfNeeded()
        await expect(page.locator('.route-map')).toHaveAttribute('data-state', 'ready')
        await page.waitForTimeout(300) // la fin du fondu de la carte
        await checkLayout(page)
        await shot('jour-activite')

        // La carte zoomée d'un cran, ses boutons et « Recentrer ».
        const figure = page.getByTestId('route-map')
        await figure.getByRole('button', { name: 'Zoomer', exact: true }).click()
        await expect(figure.getByRole('button', { name: 'Recentrer' })).toBeVisible()
        await page.waitForTimeout(400) // la fin du zoom
        await figure.screenshot({ path: `${SHOTS}/${width}-${colorScheme}-carte-zoomee.png` })

        await page.goto(`/mois/${DAY.slice(0, 7)}`)
        await expect(page.locator(`.cell[data-date="${DAY}"] .day-dot[data-metric="workout"]`)).toHaveCount(1)
        await page.waitForTimeout(300)
        await checkLayout(page)
        await shot('mois-garmin')

        await page.goto(`/annee/${DAY.slice(0, 4)}`)
        await expect(page.getByTestId('year-grid')).toHaveAttribute('data-mode', 'plusieurs')
        await page.waitForTimeout(300)
        await checkLayout(page)
        await shot('annee-garmin')
      })
    }
  })
}
