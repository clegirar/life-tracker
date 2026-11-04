import { expect, test, type APIRequestContext, type Page } from '@playwright/test'
import { addDays, apiSession, checkLayout, login, parisToday, previousDay, scrobble, send, waitForListens } from './helpers'

// Noms fictifs, propres à ce fichier : une découverte dépend de tout le journal, qui n'efface rien.
// Chaque test a sa journée et ses artistes.
const today = parisToday()
const SHOTS = process.env.E2E_SCREENSHOTS ?? 'e2e-artifacts/screenshots'
const section = (page: Page) => page.getByTestId('discoveries')
const button = (page: Page, name: string) => section(page).getByRole('button', { name: `${name} : déjà connu` })
/** Le sélecteur de la carte : il trie aussi les découvertes. */
const tab = (page: Page, name: 'Titres' | 'Artistes' | 'Albums') => page.getByTestId('music').getByRole('button', { name, exact: true }).click()
const discovered = (page: Page) => section(page).getByTestId('discoveries-count')

interface ServerRow {
  level: string
  name: string
  known_marks: { source: string; key: string }[]
}

async function serverRows(request: APIRequestContext, cookie: string, date: string): Promise<ServerRow[]> {
  const response = await request.get(`/v1/days/${date}/music`, { headers: { Cookie: cookie } })
  return (await response.json()).first_listens
}

const marksOf = async (request: APIRequestContext, cookie: string, date: string, name: string) =>
  (await serverRows(request, cookie, date)).find((row) => row.name === name)?.known_marks.length

const levelsOf = async (request: APIRequestContext, cookie: string, date: string) =>
  (await serverRows(request, cookie, date)).filter((row) => row.known_marks.length === 0).map((row) => row.level)

async function seed(request: APIRequestContext, cookie: string, date: string, artist: string, album: string, track: string) {
  await send(request, [scrobble(date, 0, { artist, album, track })])
  await waitForListens(request, cookie, date, 1)
}

test('marquer un artiste ne touche que lui, et annuler rétablit la découverte', async ({ page, request }) => {
  const date = addDays(today, -10)
  const cookie = await apiSession(request)
  await seed(request, cookie, date, 'Duo Casqué', 'Album Casqué', 'Titre Casqué')

  await login(page)
  await previousDay(page, 10)
  // Titres d'abord : seule la découverte du titre, avec son artiste.
  await expect(section(page).locator('.find')).toHaveCount(1)
  await expect(button(page, 'Titre Casqué')).toBeVisible()
  await expect(section(page).locator('.find .row-detail')).toHaveText('Duo Casqué')
  await expect(discovered(page)).toHaveText('1')
  await tab(page, 'Albums')
  await expect(button(page, 'Album Casqué')).toBeVisible()
  await expect(section(page).locator('.find .row-detail')).toHaveText('Duo Casqué')
  await tab(page, 'Artistes')
  await expect(section(page).locator('.find')).toHaveCount(1)
  await expect(section(page).locator('.find .row-detail')).toHaveCount(0)

  await button(page, 'Duo Casqué').click()
  // Le geste se voit tout de suite, puis le serveur confirme, à ce seul niveau.
  await expect(button(page, 'Duo Casqué')).toHaveAttribute('aria-pressed', 'true')
  await expect(button(page, 'Duo Casqué')).toHaveText('Marqué connu')
  await expect.poll(() => levelsOf(request, cookie, date), { timeout: 30_000 }).toEqual(['album', 'track'])
  await expect(button(page, 'Duo Casqué')).toHaveAttribute('aria-busy', 'false', { timeout: 30_000 })
  // Marqué, il n'est plus compté parmi les découvertes.
  await expect(discovered(page)).toHaveCount(0)
  await tab(page, 'Albums')
  await expect(button(page, 'Album Casqué')).toHaveAttribute('aria-pressed', 'false')
  await tab(page, 'Titres')
  await expect(button(page, 'Titre Casqué')).toHaveAttribute('aria-pressed', 'false')

  // Il tient après un rechargement : c'est le serveur qui le dit. L'adresse garde le jour.
  await page.reload()
  await tab(page, 'Artistes')
  await expect(button(page, 'Duo Casqué')).toHaveAttribute('aria-pressed', 'true')
  await expect(discovered(page)).toHaveCount(0)

  await button(page, 'Duo Casqué').click()
  await expect(button(page, 'Duo Casqué')).toHaveAttribute('aria-pressed', 'false')
  await expect.poll(() => levelsOf(request, cookie, date), { timeout: 30_000 }).toEqual(['artist', 'album', 'track'])
  await expect(discovered(page)).toHaveText('1')
})

test('un double toucher ne marque qu\'une fois', async ({ page, request }) => {
  const date = addDays(today, -11)
  const cookie = await apiSession(request)
  await seed(request, cookie, date, 'Groupe Double', 'Album Double', 'Titre Double')
  await login(page)
  await previousDay(page, 11)
  await tab(page, 'Albums')
  await button(page, 'Album Double').dblclick()
  await expect(button(page, 'Album Double')).toHaveAttribute('aria-pressed', 'true')
  await expect.poll(() => marksOf(request, cookie, date, 'Album Double'), { timeout: 30_000 }).toBe(1)
  await expect(button(page, 'Album Double')).toHaveAttribute('aria-busy', 'false', { timeout: 30_000 })
  await expect(button(page, 'Album Double')).toHaveAttribute('aria-pressed', 'true')
  expect(await marksOf(request, cookie, date, 'Album Double')).toBe(1)
})

test('un envoi perdu rend l\'état réel, le dit, et se réessaie avec la même clé', async ({ page, request }) => {
  const date = addDays(today, -12)
  const cookie = await apiSession(request)
  await seed(request, cookie, date, 'Groupe Coupé', 'Album Coupé', 'Titre Coupé')
  await login(page)
  await previousDay(page, 12)

  await page.route('**/v1/records', (route) => route.abort())
  await button(page, 'Titre Coupé').click()
  await expect(section(page).getByText("Le marquage de « Titre Coupé » n'a pas été enregistré.")).toBeVisible({ timeout: 15_000 })
  await expect(button(page, 'Titre Coupé')).toHaveAttribute('aria-pressed', 'false')

  await page.unroute('**/v1/records')
  await section(page).getByRole('button', { name: 'Réessayer' }).click()
  await expect(button(page, 'Titre Coupé')).toHaveAttribute('aria-pressed', 'true')
  await expect.poll(() => marksOf(request, cookie, date, 'Titre Coupé'), { timeout: 30_000 }).toBe(1)
  await expect(section(page).getByRole('alert')).toHaveCount(0)
})

test('une journée sans découverte le dit, sans cacher ses écoutes', async ({ page, request }) => {
  const date = addDays(today, -13)
  const cookie = await apiSession(request)
  await send(request, [scrobble(addDays(date, -1), 0, { artist: 'Groupe Connu', album: 'Album Connu', track: 'Titre Connu' })])
  await waitForListens(request, cookie, addDays(date, -1), 1)
  await seed(request, cookie, date, 'Groupe Connu', 'Album Connu', 'Titre Connu')
  await login(page)
  await previousDay(page, 13)
  await expect(page.getByTestId('listens')).toHaveText('1')
  for (const [name, text] of [
    ['Titres', 'Aucun titre découvert ce jour-là.'],
    ['Artistes', 'Aucun artiste découvert ce jour-là.'],
    ['Albums', 'Aucun album découvert ce jour-là.'],
  ] as const) {
    await tab(page, name)
    await expect(section(page).getByText(text)).toBeVisible()
  }
  // Les écoutes restent là, sous les découvertes.
  await expect(page.getByTestId('listens-list').locator('.row')).toHaveCount(1)
})

test('un album partagé par deux crédits est une seule découverte, à l\'artiste principal', async ({ page, request }) => {
  const date = addDays(today, -21)
  const cookie = await apiSession(request)
  await send(request, [
    scrobble(date, 0, { artist: 'Duo Principal & Duo Second', album: 'Album Partagé', track: 'Titre Un' }),
    scrobble(date, 1, { artist: 'Duo Principal & Autre Invité', album: 'Album Partagé', track: 'Titre Deux' }),
  ])
  await waitForListens(request, cookie, date, 2)

  await login(page)
  await previousDay(page, 21)
  await tab(page, 'Artistes')
  await expect(section(page).locator('.find')).toHaveCount(3)
  await tab(page, 'Albums')
  await expect(section(page).locator('.find')).toHaveCount(1)
  await expect(section(page).locator('.find .row-detail')).toHaveText('Duo Principal')

  await button(page, 'Album Partagé').click()
  await expect.poll(() => levelsOf(request, cookie, date), { timeout: 30_000 }).toEqual(['artist', 'artist', 'artist', 'track', 'track'])
  await expect(button(page, 'Album Partagé')).toHaveAttribute('aria-busy', 'false', { timeout: 30_000 })
  await expect(button(page, 'Album Partagé')).toHaveAttribute('aria-pressed', 'true')
})

for (const [colorScheme, daysAgo] of [['light', 15], ['dark', 16]] as const) {
  test.describe(colorScheme, () => {
    test.use({ colorScheme })

    test(`découvertes à 390 px, ${colorScheme}`, async ({ page, request }) => {
      const date = addDays(today, -daysAgo)
      const cookie = await apiSession(request)
      const long = `Un Nom Fictif Extrêmement Long ${colorScheme} Qui Ne Tient Pas Sur Une Seule Ligne`
      await send(request, [
        scrobble(date, 0, { artist: `Artiste Écran ${colorScheme}`, album: `Album Écran ${colorScheme}`, track: `Titre Écran ${colorScheme}` }),
        scrobble(date, 1, { artist: `${long}Sansespacedutoutpourvoirsilepagedéborde`, album: long, track: long }),
      ])
      await waitForListens(request, cookie, date, 2)

      await login(page)
      await previousDay(page, daysAgo)
      await expect(section(page).locator('.find')).toHaveCount(2)
      await tab(page, 'Albums')
      await button(page, `Album Écran ${colorScheme}`).click()
      await expect(button(page, `Album Écran ${colorScheme}`)).toHaveAttribute('aria-busy', 'false', { timeout: 30_000 })
      expect(page.viewportSize()!.width).toBe(390)
      await checkLayout(page)
      await page.screenshot({ path: `${SHOTS}/${colorScheme}-decouvertes.png`, fullPage: true })
    })
  })
}
