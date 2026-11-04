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

/** Une boucle fictive : un ovale d'environ 1,6 km sur 0,9. */
function loop(): [number, number, number][] {
  return Array.from({ length: 80 }, (_, i) => {
    const angle = (i / 79) * 2 * Math.PI
    return [
      Math.round((48.85 + 0.004 * Math.sin(angle)) * 1e6) / 1e6,
      Math.round((2.35 + 0.011 * (1 - Math.cos(angle))) * 1e6) / 1e6,
      i * 22,
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
  const map = card.locator('.route-map')
  await expect(map).toHaveAttribute('data-state', 'ready')
  await expect(map).toHaveAccessibleName('Parcours : course de 07:30')
  await expect(map.locator('.leaflet-overlay-pane path')).toHaveCount(3)
  await expect(card.getByRole('link', { name: '© les contributeurs d’OpenStreetMap' })).toBeVisible()
  await checkLayout(page)
})

test('un jour sans données Garmin le dit, sans zéro', async ({ page }) => {
  await openDay(page, NO_DATA)
  const card = page.getByTestId('activity')
  await expect(card).toContainText('Pas de données Garmin ce jour-là.')
  await expect(card.getByTestId('steps')).toHaveCount(0)
})

test('six métriques : toutes choisies par défaut, et Pas seule montre ses chiffres', async ({ page, request }) => {
  await seed(request, await apiSession(request))
  await login(page)
  await page.goto(`/mois/${DAY.slice(0, 7)}`)
  const filters = page.getByRole('group', { name: 'Métriques affichées' })
  for (const name of ['Cafés', 'Écoutes', 'Pas', 'Calories brûlées', 'Sommeil', 'Séances']) {
    await expect(filters.getByRole('button', { name })).toHaveAttribute('aria-pressed', 'true')
  }
  const cell = page.locator(`.cell[data-date="${DAY}"]`)
  for (const metric of ['steps', 'calories.burned', 'sleep', 'workout']) {
    await expect(cell.locator(`.day-dot[data-metric="${metric}"]`)).toHaveCount(1)
  }
  await expect(cell).toHaveAccessibleName(/2\s350 kcal.*7 h 13.*8\s412 pas.*2 séances/)
  for (const name of ['Cafés', 'Écoutes', 'Calories brûlées', 'Sommeil', 'Séances']) await filters.getByRole('button', { name }).click()
  await expect(cell.locator('.cell-value[data-metric="steps"]')).toHaveText('8,4 k')
  await expect(page.locator(`.cell[data-date="${NO_DATA}"] .cell-value`)).toHaveText(' ')
  await filters.getByRole('button', { name: 'Sommeil' }).click()
  await filters.getByRole('button', { name: 'Pas' }).click()
  await expect(cell.locator('.cell-value[data-metric="sleep"]')).toHaveText('7,2 h')
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
        await expect(page.locator('.route-map')).toHaveAttribute('data-state', 'ready')
        await page.waitForTimeout(300) // la fin du fondu de la carte
        await checkLayout(page)
        await shot('jour-activite')

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
