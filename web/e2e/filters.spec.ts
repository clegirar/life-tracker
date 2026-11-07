import { expect, test, type APIRequestContext, type Page } from '@playwright/test'
import { addDays, apiSession, checkLayout, DESKTOP, fullShot, GARMIN_TOKEN, login, noon, parisToday, scrobble, send, waitForListens } from './helpers'

// Les filtres de métriques sur le jour, et « Tout cocher » partout : un seul choix pour le mois, l'année
// et le jour. Un jour que les autres fichiers n'utilisent pas : 24 jours avant aujourd'hui. Noms fictifs.
const SHOTS = process.env.E2E_SCREENSHOTS ?? 'e2e-artifacts/screenshots'
const today = parisToday()
const DAY = addDays(today, -24)

const filters = (page: Page) => page.getByRole('group', { name: 'Métriques affichées' })
const chip = (page: Page, name: string) => filters(page).getByRole('button', { name, exact: true })
const toggleAll = (page: Page) => filters(page).locator('.filters-all')
const cell = (page: Page, date: string) => page.locator(`.cell[data-date="${date}"]`)

async function seed(request: APIRequestContext) {
  const cookie = await apiSession(request)
  await send(request, [scrobble(DAY, 0, { artist: 'Groupe Filtré', album: 'Album Filtré', track: 'Titre Filtré' })])
  await waitForListens(request, cookie, DAY, 1)
  await send(
    request,
    [
      {
        key: `summary:${DAY}:filtres`,
        kind: 'garmin.daily_summary',
        schema_version: 1,
        occurred_at: noon(DAY),
        payload: { calendar_date: DAY, steps: 6240, distance_m: 4410, calories_total: 2210, calories_active: 420, calories_resting: 1790 },
      },
    ],
    GARMIN_TOKEN,
  )
  await expect
    .poll(async () => (await (await request.get(`/v1/days/${DAY}/activity`, { headers: { Cookie: cookie } })).json()).steps?.value ?? null, {
      timeout: 15_000,
    })
    .toBe(6240)
}

async function openDay(page: Page) {
  await login(page)
  await page.goto(`/jour/${DAY}`)
  await expect(page.getByTestId('activity').getByTestId('steps')).toBeVisible()
  await expect(chip(page, 'Écoutes')).toHaveAttribute('aria-pressed', 'true')
}

test('le jour suit les filtres, et le mois aussi : un seul choix', async ({ page, request }) => {
  await seed(request)
  await openDay(page)
  await expect(page.getByTestId('music')).toBeVisible()

  // Décocher les écoutes masque la carte Musique…
  await chip(page, 'Écoutes').click()
  await expect(chip(page, 'Écoutes')).toHaveAttribute('aria-pressed', 'false')
  await expect(page.getByTestId('music')).toHaveCount(0)
  await expect(page.getByTestId('activity')).toBeVisible()
  // … et ses points dans le mois.
  await page.getByRole('button', { name: /^Voir le mois/ }).click()
  await expect(chip(page, 'Écoutes')).toHaveAttribute('aria-pressed', 'false')
  await expect(cell(page, DAY).locator('.day-dot[data-metric="steps"]')).toHaveCount(1)
  await expect(cell(page, DAY).locator('.day-dot[data-metric="music.listen"]')).toHaveCount(0)
  await page.goBack()

  // « Activité » coche et décoche les quatre métriques Garmin ensemble : la carte entière part.
  await chip(page, 'Activité').click()
  await expect(chip(page, 'Activité')).toHaveAttribute('aria-pressed', 'false')
  const activity = page.getByTestId('activity')
  await expect(activity).toHaveCount(0)
  await expect(page.locator('.card-coffee')).toBeVisible()
  await checkLayout(page)

  // Le choix survit au rechargement.
  await page.reload()
  await expect(chip(page, 'Activité')).toHaveAttribute('aria-pressed', 'false')
  await expect(page.getByTestId('music')).toHaveCount(0)
  await expect(activity).toHaveCount(0)
  await expect(page.locator('.card-coffee')).toBeVisible()

  // Recochée : les quatre blocs reviennent.
  await chip(page, 'Activité').click()
  await expect(activity.getByTestId('steps')).toBeVisible()
  await expect(activity.getByTestId('calories')).toBeVisible()
  await expect(activity.getByTestId('workouts')).toBeVisible()
})

test('un ancien choix d’une partie des métriques Garmin allume « Activité » et montre ses quatre blocs', async ({ page, request }) => {
  await seed(request)
  await login(page)
  // Un choix gardé d'avant la pastille : le café et le sommeil seulement.
  await page.evaluate(() => window.localStorage.setItem('calendar.metrics', JSON.stringify(['coffee', 'sleep'])))
  await page.goto(`/jour/${DAY}`)
  const activity = page.getByTestId('activity')
  await expect(chip(page, 'Activité')).toHaveAttribute('aria-pressed', 'true')
  await expect(chip(page, 'Écoutes')).toHaveAttribute('aria-pressed', 'false')
  await expect(activity.getByTestId('steps')).toBeVisible()
  await expect(activity.getByTestId('calories')).toBeVisible()
  // Un appui éteint les quatre, un autre les rallume.
  await chip(page, 'Activité').click()
  await expect(activity).toHaveCount(0)
  const stored = () => page.evaluate(() => window.localStorage.getItem('calendar.metrics'))
  await expect.poll(stored).toBe('["coffee"]')
  await chip(page, 'Activité').click()
  await expect(activity.getByTestId('steps')).toBeVisible()
  await expect.poll(stored).toBe('["coffee","steps","workout","sleep","calories.burned"]')
})

test('tout décocher vide la vue et invite à choisir, sur le jour, le mois et l’année', async ({ page, request }) => {
  await seed(request)
  await openDay(page)

  await expect(toggleAll(page)).toHaveText('Tout décocher')
  await toggleAll(page).click()
  for (const name of ['Cafés', 'Écoutes', 'Activité']) await expect(chip(page, name)).toHaveAttribute('aria-pressed', 'false')
  await expect(toggleAll(page)).toHaveText('Tout cocher')
  await expect(page.getByTestId('no-metric')).toHaveText('Choisis au moins une métrique.')
  await expect(page.locator('.cards > *')).toHaveCount(0)
  await checkLayout(page)

  // Le mois : le même choix, ni point ni chiffre, l'invitation à la place de la légende.
  await page.getByRole('button', { name: /^Voir le mois/ }).click()
  await expect(toggleAll(page)).toHaveText('Tout cocher')
  await expect(page.getByTestId('month-grid').locator('.day-dot, .cell-value, [data-nodata]')).toHaveCount(0)
  await expect(page.getByTestId('no-metric')).toHaveText('Choisis au moins une métrique.')
  await expect(cell(page, DAY)).toBeEnabled()
  await checkLayout(page)

  // L'année : de même ; « Tout cocher » ramène tout.
  await page.getByRole('button', { name: /^Voir l'année/ }).click()
  const grid = page.getByTestId('year-grid')
  await expect(grid).toHaveAttribute('data-mode', 'aucune')
  await expect(grid.locator('.day-dot, .dot, [data-nodata]')).toHaveCount(0)
  await expect(page.getByTestId('no-metric')).toBeVisible()
  await toggleAll(page).click()
  await expect(grid).toHaveAttribute('data-mode', 'plusieurs')
  await expect(toggleAll(page)).toHaveText('Tout décocher')
  await expect(page.getByTestId('no-metric')).toHaveCount(0)

  // Une seule recochée : « Tout cocher » reste proposé.
  await page.goto(`/jour/${DAY}`)
  await expect(page.getByTestId('music')).toBeVisible()
  await toggleAll(page).click()
  await chip(page, 'Écoutes').click()
  await expect(toggleAll(page)).toHaveText('Tout cocher')
  await expect(page.getByTestId('music')).toBeVisible()
  await expect(page.getByTestId('activity')).toHaveCount(0)
  await expect(page.getByTestId('no-metric')).toHaveCount(0)
})

for (const colorScheme of ['light', 'dark'] as const) {
  test.describe(colorScheme, () => {
    test.use({ colorScheme })

    for (const [width, viewport] of [['390', { width: 390, height: 844 }], ['1280', DESKTOP]] as const) {
      test(`filtres du jour et du mois à ${width} px, ${colorScheme}`, async ({ page, request }) => {
        await seed(request)
        await page.setViewportSize(viewport)
        const shot = async (name: string) => {
          // Le pointeur hors des commandes : la capture pleine page d'iPhone réveille un instant le survol.
          await page.mouse.move(viewport.width - 1, viewport.height - 1)
          await fullShot(page, `${SHOTS}/${width}-${colorScheme}-${name}.png`)
        }
        await openDay(page)

        // Le mois, tout coché : le bouton dit « Tout décocher ».
        await page.getByRole('button', { name: /^Voir le mois/ }).click()
        await expect(cell(page, DAY).locator('.day-dot[data-metric="steps"]')).toHaveCount(1)
        await page.waitForTimeout(300) // la fin de l'arrivée
        await checkLayout(page)
        await shot('mois-filtres')

        // Le jour, deux pastilles : le café et l'activité.
        await page.goBack()
        await expect(page.getByTestId('music')).toBeVisible()
        for (const name of ['Écoutes', 'Vidéos YouTube', 'Twitch', 'Jeux']) await chip(page, name).click()
        await expect(page.getByTestId('music')).toHaveCount(0)
        await expect(page.getByTestId('activity').getByTestId('steps')).toBeVisible()
        await page.waitForTimeout(200) // la fin du filtre
        await checkLayout(page)
        await shot('jour-filtres')

        // Rien de coché : l'invitation, sur le jour puis dans le mois.
        await toggleAll(page).click()
        await toggleAll(page).click()
        await expect(page.getByTestId('no-metric')).toBeVisible()
        await page.waitForTimeout(200)
        await checkLayout(page)
        await shot('jour-sans-metrique')
        await page.getByRole('button', { name: /^Voir le mois/ }).click()
        await expect(page.getByTestId('no-metric')).toBeVisible()
        await page.waitForTimeout(300)
        await checkLayout(page)
        await shot('mois-sans-metrique')
      })
    }
  })
}
