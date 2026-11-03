import { expect, test, type APIRequestContext, type Page } from '@playwright/test'
import { addDays, API_URL, apiSession, checkLayout, DESKTOP, IOS_TOKEN, login, noon, parisToday, scrobble, send, waitForListens } from './helpers'

// La vue de l'année. Des jours que les autres fichiers n'utilisent pas : 26 à 28. Noms fictifs.
const SHOTS = process.env.E2E_SCREENSHOTS ?? 'e2e-artifacts/screenshots'
const today = parisToday()
const yearOf = (date: string) => date.slice(0, 4)
const monthOf = (date: string) => date.slice(0, 7)
const path = (url: string) => new URL(url).pathname
const monthLabel = (month: string) => {
  const name = new Intl.DateTimeFormat('fr-FR', { timeZone: 'UTC', month: 'long' }).format(new Date(`${month}-15T12:00:00Z`))
  return `${name.charAt(0).toUpperCase()}${name.slice(1)} ${month.slice(0, 4)}`
}
// Un jour de l'année, pastille (une métrique) ou numéro et points (plusieurs).
const yearDay = (page: Page, date: string) => page.getByTestId('year-grid').locator(`[data-date="${date}"]`)
const dot = (page: Page, date: string) => page.locator(`.dot[data-date="${date}"]`)

// Le premier café du journal : la veille n'a pas de données, le lendemain est un vrai zéro.
const FIRST_CUP = addDays(today, -28)

async function seedFirstCup(request: APIRequestContext, cookie: string) {
  const response = await request.post(`${API_URL}/v1/records`, {
    headers: { Authorization: `Bearer ${IOS_TOKEN}` },
    data: { records: [{ key: `annee-premier-cafe-${FIRST_CUP}`, kind: 'coffee.cup', schema_version: 1, occurred_at: noon(FIRST_CUP), payload: {} }] },
  })
  expect(response.status()).toBe(200)
  await expect
    .poll(async () => (await (await request.get(`/v1/days/${FIRST_CUP}`, { headers: { Cookie: cookie } })).json()).count, { timeout: 10_000 })
    .toBe(1)
}

test('jour, mois, année, puis mois : chaque vue a son adresse et son retour', async ({ page }) => {
  await login(page)
  await page.getByRole('button', { name: /^Voir le mois/ }).click()
  await expect(page.getByTestId('month-grid')).toBeVisible()
  await page.getByRole('button', { name: `Voir l'année ${yearOf(today)}` }).click()
  await expect(page.getByRole('heading', { name: yearOf(today) })).toBeVisible()
  expect(path(page.url())).toBe(`/annee/${yearOf(today)}`)
  await expect(page.getByRole('button', { name: 'Année suivante' })).toBeDisabled()
  // Les jours futurs n'apparaissent pas, et aujourd'hui est marqué.
  await expect(yearDay(page, addDays(today, 1))).toHaveCount(0)
  await expect(yearDay(page, today)).toHaveAttribute('data-today', '')

  const current = page.getByRole('button', { name: monthLabel(monthOf(today)), exact: true })
  await expect(current).toHaveAttribute('aria-current', 'date')
  await current.click()
  await expect(page.getByTestId('month-grid')).toBeVisible()
  expect(path(page.url())).toBe(`/mois/${monthOf(today)}`)

  await page.goBack()
  await expect(page.getByTestId('year-grid')).toBeVisible()
  await page.getByRole('button', { name: 'Année précédente' }).click()
  const previous = String(Number(yearOf(today)) - 1)
  await expect(page.getByRole('heading', { name: previous })).toBeVisible()
  await expect(page.getByRole('button', { name: 'Année suivante' })).toBeEnabled()
  // Toute l'année passée est là ; décembre ouvre son mois.
  const leap = new Date(Date.UTC(Number(previous), 1, 29)).getUTCMonth() === 1
  await expect(page.getByTestId('year-grid').locator('[data-date]')).toHaveCount(leap ? 366 : 365)
  await page.getByRole('button', { name: monthLabel(`${previous}-12`), exact: true }).click()
  expect(path(page.url())).toBe(`/mois/${previous}-12`)
})

const chip = (page: Page, name: 'Cafés' | 'Écoutes') => page.getByRole('group', { name: 'Métriques affichées' }).getByRole('button', { name })
const cell = (page: Page, date: string) => page.locator(`.cell[data-date="${date}"]`)

const ALL = ['coffee', 'music.listen']
type ServerDay = { date: string; values: Record<string, number | null> }

/** Les jours passés de l'année, lus au serveur : de quoi trouver un jour sans données et un jour sans activité. */
async function pastDays(request: APIRequestContext, cookie: string, year: string): Promise<ServerDay[]> {
  const response = await request.get(`/v1/years/${year}`, { headers: { Cookie: cookie } })
  expect(response.status()).toBe(200)
  return ((await response.json()).days as ServerDay[]).filter((day) => day.date <= today)
}
const value = (day: ServerDay, metric: string) => day.values[metric] ?? null

test('une métrique : chiffres du mois et intensité de l’année ; plusieurs : des points dans les deux', async ({ page, request }) => {
  const cookie = await apiSession(request)
  await seedFirstCup(request, cookie)
  // Un jour d'écoutes : la musique a sa propre échelle.
  const busy = addDays(today, -27)
  await send(request, Array.from({ length: 9 }, (_, i) => scrobble(busy, i, { artist: 'Groupe Annuel', track: `Titre annuel ${i}` })))
  await waitForListens(request, cookie, busy, 9)
  const days = await pastDays(request, cookie, yearOf(FIRST_CUP))
  // Pas de données pour aucune des deux, et un jour connu sans activité.
  const empty = days.find((day) => ALL.every((m) => value(day, m) === null))
  const quiet = days.find((day) => ALL.some((m) => value(day, m) !== null) && ALL.every((m) => (value(day, m) ?? 0) === 0))
  expect(empty, 'un jour sans données').toBeDefined()
  expect(quiet, 'un jour sans activité').toBeDefined()

  await login(page)
  await page.goto(`/annee/${yearOf(FIRST_CUP)}`)
  const grid = page.getByTestId('year-grid')
  // Par défaut, toutes les métriques : un point par métrique active, dans sa couleur.
  await expect(chip(page, 'Cafés')).toHaveAttribute('aria-pressed', 'true')
  await expect(chip(page, 'Écoutes')).toHaveAttribute('aria-pressed', 'true')
  await expect(grid).toHaveAttribute('data-mode', 'plusieurs')
  await expect(grid.locator('.dot')).toHaveCount(0)
  await expect(yearDay(page, FIRST_CUP).locator('.day-dot[data-metric="coffee"]')).toHaveCount(1)
  await expect(yearDay(page, busy).locator('.day-dot[data-metric="music.listen"]')).toHaveCount(1)
  // Jamais plus de trois points par jour, et dans l'ordre du registre.
  const perDay = await grid.locator('.day-dots-year').evaluateAll((all) => all.map((el) => [...el.children].map((dot) => dot.getAttribute('data-metric'))))
  for (const dots of perDay) {
    expect(dots.length).toBeLessThanOrEqual(3)
    expect(dots).toEqual(ALL.filter((m) => dots.includes(m)))
  }
  // Sans données : numéro grisé et aucun point. Sans activité : numéro normal, aucun point.
  await expect(yearDay(page, empty!.date)).toHaveAttribute('data-nodata', '')
  await expect(yearDay(page, empty!.date).locator('.day-dot')).toHaveCount(0)
  await expect(yearDay(page, quiet!.date)).not.toHaveAttribute('data-nodata')
  await expect(yearDay(page, quiet!.date).locator('.day-dot')).toHaveCount(0)
  await expect(page.getByRole('button', { name: monthLabel(monthOf(FIRST_CUP)), exact: true })).toHaveAccessibleDescription(/cafés?.*;.*écoutes?/)
  await expect(page.getByRole('region', { name: 'Légende' })).toContainText('Un point par activité du jour. Numéro grisé : pas de données.')
  await checkLayout(page)

  // Une seule : retour à l'intensité de sa couleur, sans points.
  await chip(page, 'Écoutes').click()
  await expect(chip(page, 'Écoutes')).toHaveAttribute('aria-pressed', 'false')
  await expect(grid).toHaveAttribute('data-mode', 'une')
  await expect(grid.locator('.day-dot')).toHaveCount(0)
  await expect(dot(page, addDays(FIRST_CUP, -1))).toHaveAttribute('data-heat', 'none')
  await expect(dot(page, FIRST_CUP)).toHaveAttribute('data-heat', /[1-4]/)
  await expect(dot(page, addDays(FIRST_CUP, 1))).toHaveAttribute('data-heat', '0')
  await expect(page.getByRole('region', { name: 'Légende' })).toContainText('Pâle : rien ce jour-là')
  // Le résumé du mois se lit, les pastilles sont décoratives.
  await expect(page.getByRole('button', { name: monthLabel(monthOf(FIRST_CUP)), exact: true })).toHaveAccessibleDescription(/cafés?/)
  // La dernière métrique choisie ne se retire pas.
  await chip(page, 'Cafés').click()
  await expect(chip(page, 'Cafés')).toHaveAttribute('aria-pressed', 'true')
  await expect(grid).toHaveAttribute('data-mode', 'une')

  // Le même choix vaut pour le mois : une métrique, ses chiffres.
  await page.getByRole('button', { name: monthLabel(monthOf(FIRST_CUP)), exact: true }).click()
  await expect(chip(page, 'Écoutes')).toHaveAttribute('aria-pressed', 'false')
  const coffee = (date: string) => cell(page, date).locator('.cell-value[data-metric="coffee"]')
  await expect(cell(page, FIRST_CUP).locator('.cell-value')).toHaveCount(1)
  await expect(coffee(FIRST_CUP)).toHaveText('1')
  await expect(coffee(addDays(FIRST_CUP, -1))).toHaveText(' ')
  await expect(page.getByTestId('month-grid').locator('[data-heat], .day-dot')).toHaveCount(0)
  await expect(page.getByRole('region', { name: 'Légende' })).toContainText('Sans chiffre : pas de données')
  await checkLayout(page)

  // Changé dans le mois, il vaut pour l'année, et survit au rechargement.
  await chip(page, 'Écoutes').click()
  await chip(page, 'Cafés').click()
  await expect(cell(page, busy).locator('.cell-value[data-metric="music.listen"]')).toHaveText('9')
  await page.getByRole('button', { name: `Voir l'année ${yearOf(FIRST_CUP)}` }).click()
  await expect(chip(page, 'Cafés')).toHaveAttribute('aria-pressed', 'false')
  await expect(dot(page, busy)).toHaveAttribute('data-heat', /[1-4]/)
  await page.reload()
  await expect(chip(page, 'Écoutes')).toHaveAttribute('aria-pressed', 'true')
  await expect(chip(page, 'Cafés')).toHaveAttribute('aria-pressed', 'false')

  // De nouveau les deux : des points dans le mois, plus de chiffres.
  await page.goto(`/mois/${monthOf(busy)}`)
  await chip(page, 'Cafés').click()
  await expect(page.getByTestId('month-grid').locator('.cell-value')).toHaveCount(0)
  await expect(cell(page, busy).locator('.day-dot[data-metric="music.listen"]')).toHaveCount(1)
  await expect(cell(page, busy)).toHaveAccessibleName(/9 écoutes/)
  await expect(cell(page, FIRST_CUP).locator('.day-dot[data-metric="coffee"]')).toHaveCount(1)
  await expect(page.getByRole('region', { name: 'Légende' })).toContainText('Numéro grisé : pas de données')
  await checkLayout(page)
  // Le jour sans données et le jour sans activité, dans leur mois.
  await page.goto(`/mois/${monthOf(empty!.date)}`)
  await expect(cell(page, empty!.date)).toHaveAttribute('data-nodata', '')
  await expect(cell(page, empty!.date)).toHaveAccessibleName(/^[^:]+ : pas de données/)
  await expect(cell(page, empty!.date).locator('.day-dot')).toHaveCount(0)
  await page.goto(`/mois/${monthOf(quiet!.date)}`)
  await expect(cell(page, quiet!.date)).not.toHaveAttribute('data-nodata')
  await expect(cell(page, quiet!.date).locator('.day-dot')).toHaveCount(0)
  await expect(cell(page, quiet!.date)).not.toHaveAccessibleName(/^[^:]+ : pas de données/)
})

test('une année future ou mal écrite ramène à aujourd’hui', async ({ page }) => {
  await login(page)
  for (const invalid of [`/annee/${Number(yearOf(today)) + 1}`, '/annee/26']) {
    await page.goto(invalid)
    await expect(page.getByRole('heading', { name: "Aujourd'hui" })).toBeVisible()
    expect(path(page.url())).toBe('/')
  }
})

for (const colorScheme of ['light', 'dark'] as const) {
  test.describe(colorScheme, () => {
    test.use({ colorScheme })

    for (const [label, viewport] of [['390', { width: 390, height: 844 }], ['large', DESKTOP]] as const) {
      test(`année et mois à ${label === '390' ? '390 px' : '1280 px'}, ${colorScheme}`, async ({ page, request }) => {
        const cookie = await apiSession(request)
        await seedFirstCup(request, cookie)
        // Quelques jours de cafés, à des niveaux différents, jamais avant le premier.
        for (const [daysAgo, cups] of [[26, 1], [27, 3]] as const) {
          const date = addDays(today, -daysAgo)
          const response = await request.post(`${API_URL}/v1/records`, {
            headers: { Authorization: `Bearer ${IOS_TOKEN}` },
            data: {
              records: Array.from({ length: cups }, (_, i) => ({
                key: `annee-${date}-${i}`,
                kind: 'coffee.cup',
                schema_version: 1,
                occurred_at: noon(date, i),
                payload: {},
              })),
            },
          })
          expect(response.status()).toBe(200)
          await expect
            .poll(async () => (await (await request.get(`/v1/days/${date}`, { headers: { Cookie: cookie } })).json()).count, { timeout: 10_000 })
            .toBe(cups)
        }
        await page.setViewportSize(viewport)
        await login(page)
        const width = label === '390' ? '390' : '1280'
        const shot = (name: string) => page.screenshot({ path: `${SHOTS}/${width}-${colorScheme}-${name}.png`, fullPage: true })

        await page.goto(`/annee/${yearOf(today)}`)
        await expect(page.getByTestId('year-grid')).toHaveAttribute('data-mode', 'plusieurs')
        await expect(yearDay(page, addDays(today, -27)).locator('.day-dot[data-metric="coffee"]')).toHaveCount(1)
        await page.waitForTimeout(300) // la fin de l'arrivée
        await checkLayout(page)
        await shot('annee-plusieurs')
        await chip(page, 'Écoutes').click()
        await expect(dot(page, today)).toHaveAttribute('data-heat', /.+/)
        await page.waitForTimeout(200) // la fin du filtre
        await checkLayout(page)
        await shot('annee-une')

        await page.goto(`/mois/${monthOf(addDays(today, -26))}`)
        await expect(cell(page, addDays(today, -26)).locator('.cell-value')).toHaveCount(1)
        await page.waitForTimeout(300)
        await checkLayout(page)
        await shot('mois-une')
        await chip(page, 'Écoutes').click()
        await expect(cell(page, addDays(today, -26)).locator('.day-dot[data-metric="coffee"]')).toHaveCount(1)
        await page.waitForTimeout(200) // la fin du filtre
        await checkLayout(page)
        await shot('mois-plusieurs')
      })
    }
  })
}
