import { expect, test } from '@playwright/test'
import { addDays, checkLayout, DESKTOP, fullShot, login, openMenu, parisToday } from './helpers'

// La barre latérale, le tiroir sur iPhone et les adresses des pages.
const SHOTS = process.env.E2E_SCREENSHOTS ?? 'e2e-artifacts/screenshots'
const today = parisToday()
const monthOf = (date: string) => date.slice(0, 7)
const path = (url: string) => new URL(url).pathname

test('le tiroir s’ouvre, prend le focus, et se ferme par Échap, le voile ou une entrée', async ({ page }) => {
  await login(page)
  const menu = page.getByRole('button', { name: 'Menu' })
  const drawer = page.getByRole('dialog', { name: 'Menu' })
  await expect(drawer).toBeHidden()
  await expect(menu).toHaveAttribute('aria-expanded', 'false')

  await menu.click()
  await expect(drawer).toBeVisible()
  await expect(menu).toHaveAttribute('aria-expanded', 'true')
  await expect(drawer.getByRole('link', { name: 'Calendrier' })).toBeFocused()
  // Le reste de la page ne reçoit plus rien.
  await expect(page.locator('.content')).toHaveAttribute('inert', '')
  await page.waitForTimeout(350) // la fin de l'ouverture
  await checkLayout(page)

  await page.keyboard.press('Escape')
  await expect(drawer).toBeHidden()
  await expect(menu).toBeFocused()
  await expect(page.locator('.content')).not.toHaveAttribute('inert', '')

  await menu.click()
  await expect(drawer).toBeVisible()
  await page.locator('.scrim').click({ position: { x: 370, y: 400 } })
  await expect(drawer).toBeHidden()
  await expect(menu).toBeFocused()

  // Une entrée ferme le tiroir et mène à sa page.
  await page.getByRole('button', { name: 'Jour précédent' }).click()
  await expect(page).toHaveURL(/\/jour\//)
  await menu.click()
  await drawer.getByRole('link', { name: 'Calendrier' }).click()
  await expect(drawer).toBeHidden()
  expect(path(page.url())).toBe('/')
  await expect(page.getByRole('heading', { name: "Aujourd'hui" })).toBeVisible()
})

test('Retour revient au mois, puis au jour', async ({ page }) => {
  const yesterday = addDays(today, -1)
  await login(page)
  expect(path(page.url())).toBe('/')
  await page.getByRole('button', { name: 'Jour précédent' }).click()
  await expect(page.getByRole('heading', { name: 'Hier' })).toBeVisible()
  expect(path(page.url())).toBe(`/jour/${yesterday}`)

  await page.getByRole('button', { name: /^Voir le mois/ }).click()
  await expect(page.getByTestId('month-grid')).toBeVisible()
  expect(path(page.url())).toBe(`/mois/${monthOf(yesterday)}`)
  // Le jour d'où l'on vient est sélectionné.
  await expect(page.locator(`.cell[data-date="${yesterday}"]`)).toHaveAttribute('aria-pressed', 'true')

  await page.goBack()
  await expect(page.getByRole('heading', { name: 'Hier' })).toBeVisible()
  expect(path(page.url())).toBe(`/jour/${yesterday}`)
  await page.goBack()
  await expect(page.getByRole('heading', { name: "Aujourd'hui" })).toBeVisible()
  await page.goForward()
  await expect(page.getByRole('heading', { name: 'Hier' })).toBeVisible()
})

test('une adresse s’ouvre directement et résiste au rechargement ; une adresse invalide ramène à aujourd’hui', async ({ page }) => {
  const date = addDays(today, -3)
  await login(page)
  await page.goto(`/jour/${date}`)
  await expect(page.getByRole('button', { name: 'Jour suivant' })).toBeEnabled()
  await page.getByRole('button', { name: 'Jour suivant' }).click()
  expect(path(page.url())).toBe(`/jour/${addDays(date, 1)}`)
  await page.reload()
  expect(path(page.url())).toBe(`/jour/${addDays(date, 1)}`)
  await expect(page.getByRole('button', { name: 'Jour suivant' })).toBeEnabled()

  await page.goto(`/mois/${monthOf(today)}`)
  await expect(page.getByTestId('month-grid')).toBeVisible()
  await page.reload()
  await expect(page.getByTestId('month-grid')).toBeVisible()
  expect(path(page.url())).toBe(`/mois/${monthOf(today)}`)

  for (const invalid of [`/jour/${addDays(today, 1)}`, '/jour/2026-02-30', '/mois/2026-13', `/mois/${Number(today.slice(0, 4)) + 1}-01`, '/inconnu']) {
    await page.goto(invalid)
    await expect(page.getByRole('heading', { name: "Aujourd'hui" })).toBeVisible()
    expect(path(page.url()), invalid).toBe('/')
  }
})

test('une adresse sans session demande la connexion, puis ouvre la page', async ({ page }) => {
  const date = addDays(today, -2)
  await page.goto(`/jour/${date}`)
  await expect(page.getByRole('heading', { name: 'Connexion' })).toBeVisible()
  await page.getByLabel('Identifiant').fill(process.env.E2E_USER!)
  await page.getByLabel('Mot de passe').fill(process.env.E2E_PASSWORD!)
  await page.getByRole('button', { name: 'Se connecter' }).click()
  await expect(page.getByRole('button', { name: 'Jour suivant' })).toBeEnabled()
  expect(path(page.url())).toBe(`/jour/${date}`)
})

for (const colorScheme of ['light', 'dark'] as const) {
  test.describe(colorScheme, () => {
    test.use({ colorScheme })

    test(`tiroir à 390 px, ${colorScheme}`, async ({ page }) => {
      await login(page)
      expect(page.viewportSize()!.width).toBe(390)
      await openMenu(page)
      await page.waitForTimeout(350) // la fin de l'ouverture
      await expect(page.getByRole('dialog', { name: 'Menu' }).getByTestId('logo')).toBeVisible()
      await checkLayout(page)
      await page.screenshot({ path: `${SHOTS}/${colorScheme}-tiroir.png` })
    })

    test.describe('écran large', () => {
      test.use({ viewport: DESKTOP, isMobile: false, hasTouch: false })

      test(`barre latérale à 1280 px, ${colorScheme}`, async ({ page }) => {
        await login(page)
        const sidebar = page.getByRole('complementary', { name: 'Menu' })
        await expect(sidebar).toBeVisible()
        await expect(sidebar.getByRole('link', { name: 'Calendrier' })).toHaveAttribute('aria-current', 'page')
        // Le logo à côté du titre, décoratif : le nom de l'app reste le seul texte lu.
        const title = sidebar.locator('.sidebar-title')
        await expect(title).toHaveText('Life tracker')
        const logo = title.getByTestId('logo')
        await expect(logo).toBeVisible()
        await expect(logo).toHaveAttribute('aria-hidden', 'true')
        const [logoBox, titleBox] = [(await logo.boundingBox())!, (await title.boundingBox())!]
        expect(logoBox.width).toBe(28)
        // Centré sur la ligne du titre.
        expect(Math.abs(logoBox.y + logoBox.height / 2 - (titleBox.y + 4 + 14))).toBeLessThanOrEqual(1)
        await expect(page.getByRole('button', { name: 'Menu' })).toHaveCount(0)
        await expect(page.locator('.content')).not.toHaveAttribute('inert', '')
        // La barre ne recouvre pas la page : elle a sa colonne.
        const bar = (await sidebar.boundingBox())!
        const main = (await page.locator('main').boundingBox())!
        expect(main.x).toBeGreaterThanOrEqual(bar.x + bar.width)
        await checkLayout(page)
        await fullShot(page, `${SHOTS}/${colorScheme}-large-jour.png`)

        await page.getByRole('button', { name: /^Voir le mois/ }).click()
        await expect(page.getByTestId('month-grid')).toBeVisible()
        await page.waitForTimeout(300) // la fin de l'arrivée
        await checkLayout(page)
        await fullShot(page, `${SHOTS}/${colorScheme}-large-mois.png`)
      })
    })
  })
}
