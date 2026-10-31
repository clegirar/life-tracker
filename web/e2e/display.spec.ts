import { expect, test, type Page } from '@playwright/test'
import { apiSession, clearDay, count, login, parisToday, waitForServerCount } from './helpers'

// Captures à la largeur d'un iPhone, en clair et en sombre. Elles restent hors du dépôt.
const SHOTS = process.env.E2E_SCREENSHOTS ?? 'e2e-artifacts/screenshots'
const today = parisToday()

async function checkLayout(page: Page) {
  const problems = await page.evaluate(() => {
    const out: string[] = []
    const root = document.documentElement
    if (root.scrollWidth > window.innerWidth) out.push(`défilement horizontal : ${root.scrollWidth}`)
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

for (const colorScheme of ['light', 'dark'] as const) {
  test.describe(colorScheme, () => {
    test.use({ colorScheme })

    test(`iPhone 390 px, ${colorScheme}`, async ({ page, request }) => {
      const cookie = await apiSession(request)
      await clearDay(request, cookie, today)
      const shot = (name: string) => page.screenshot({ path: `${SHOTS}/${colorScheme}-${name}.png`, fullPage: true })

      await page.goto('/')
      await expect(page.getByRole('heading', { name: 'Connexion' })).toBeVisible()
      expect(page.viewportSize()!.width).toBe(390)
      await checkLayout(page)
      await shot('1-connexion')

      await login(page)
      await expect(count(page)).toHaveText('0')
      await checkLayout(page)
      await shot('2-vide')

      await page.getByRole('button', { name: 'Noter à une autre heure' }).click()
      await expect(page.getByLabel('Heure du café')).toBeVisible()
      await checkLayout(page)
      await shot('3-heure')
      await page.getByRole('button', { name: 'Fermer' }).click()
      for (let i = 0; i < 3; i++) await page.getByRole('button', { name: 'Ajouter un café maintenant' }).click()
      await expect(count(page)).toHaveText('3')
      await waitForServerCount(request, cookie, today, 3)
      await page.waitForTimeout(600) // la fin du ressort
      await checkLayout(page)
      await shot('4-cafes')

      await page.route('**/v1/records', (route) => route.abort('internetdisconnected'))
      await page.getByRole('button', { name: /^Annuler le café de/ }).first().click()
      await expect(page.getByRole('alert')).toBeVisible({ timeout: 10_000 })
      await checkLayout(page)
      await shot('5-echec')
      await page.unroute('**/v1/records')

      await page.getByRole('button', { name: 'Jour précédent' }).click()
      await expect(page.getByRole('heading', { name: 'Hier' })).toBeVisible()
      await checkLayout(page)
      await shot('6-hier')

      await clearDay(request, cookie, today)
    })
  })
}
