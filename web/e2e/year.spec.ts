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
  // Les jours futurs n'apparaissent pas, et aujourd'hui est entouré.
  await expect(dot(page, addDays(today, 1))).toHaveCount(0)
  await expect(dot(page, today)).toHaveAttribute('data-today', '')

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
  await expect(page.getByTestId('year-grid').locator('.dot')).toHaveCount(leap ? 366 : 365)
  await page.getByRole('button', { name: monthLabel(`${previous}-12`), exact: true }).click()
  expect(path(page.url())).toBe(`/mois/${previous}-12`)
})

test('une pastille vide n’a pas de données, une pastille pâle est un zéro, et la métrique se choisit', async ({ page, request }) => {
  const cookie = await apiSession(request)
  await seedFirstCup(request, cookie)
  // Un jour d'écoutes : la musique a sa propre échelle.
  const busy = addDays(today, -27)
  await send(request, Array.from({ length: 9 }, (_, i) => scrobble(busy, i, { artist: 'Groupe Annuel', track: `Titre annuel ${i}` })))
  await waitForListens(request, cookie, busy, 9)

  await login(page)
  await page.goto(`/annee/${yearOf(FIRST_CUP)}`)
  await expect(page.getByTestId('year-grid')).toHaveAttribute('data-metric', 'coffee')
  await expect(page.getByRole('button', { name: 'Cafés' })).toHaveAttribute('aria-pressed', 'true')
  await expect(dot(page, addDays(FIRST_CUP, -1))).toHaveAttribute('data-heat', 'none')
  await expect(dot(page, FIRST_CUP)).not.toHaveAttribute('data-heat', /none|0/)
  await expect(dot(page, addDays(FIRST_CUP, 1))).toHaveAttribute('data-heat', '0')
  // Le résumé du mois se lit, les pastilles sont décoratives.
  await expect(page.getByRole('button', { name: monthLabel(monthOf(FIRST_CUP)), exact: true })).toHaveAccessibleDescription(/cafés?/)

  await page.getByRole('button', { name: 'Écoutes' }).click()
  await expect(page.getByTestId('year-grid')).toHaveAttribute('data-metric', 'music.listen')
  await expect(dot(page, busy)).toHaveAttribute('data-heat', /[1-4]/)
  // Le choix est gardé d'une visite à l'autre.
  await page.reload()
  await expect(page.getByRole('button', { name: 'Écoutes' })).toHaveAttribute('aria-pressed', 'true')
  await page.getByRole('button', { name: 'Cafés' }).click()
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
      test(`année à ${label === '390' ? '390 px' : '1280 px'}, ${colorScheme}`, async ({ page, request }) => {
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
        }
        await page.setViewportSize(viewport)
        await login(page)
        await page.goto(`/annee/${yearOf(today)}`)
        await expect(dot(page, today)).toHaveAttribute('data-heat', /.+/)
        await page.waitForTimeout(300) // la fin de l'arrivée
        await checkLayout(page)
        const name = `${colorScheme}-${label === '390' ? '' : 'large-'}annee`
        await page.screenshot({ path: `${SHOTS}/${name}-cafes.png`, fullPage: true })
        await page.getByRole('button', { name: 'Écoutes' }).click()
        await checkLayout(page)
        await page.screenshot({ path: `${SHOTS}/${name}-musique.png`, fullPage: true })
        await page.getByRole('button', { name: 'Cafés' }).click()
      })
    }
  })
}
