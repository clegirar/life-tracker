import { expect, test, type Page, type Request } from '@playwright/test'
import {
  addDays,
  API_URL,
  apiSession,
  clearDay,
  count,
  IOS_TOKEN,
  login,
  parisToday,
  PASSWORD,
  serverDay,
  USER,
  waitForServerCount,
} from './helpers'

const today = parisToday()
const add = (page: Page) => page.getByRole('button', { name: 'Ajouter un café maintenant' })
const cancelButtons = (page: Page) => page.getByRole('button', { name: /^Annuler le café de/ })

type Sent = { key: string; kind: string; occurred_at: string; payload: Record<string, string> }
const sentRecord = (request: Request): Sent => request.postDataJSON().records[0]

let cookie: string

test.beforeEach(async ({ request }) => {
  cookie = await apiSession(request)
  await clearDay(request, cookie, today)
  await clearDay(request, cookie, addDays(today, -1))
})

test('parcours complet : connexion, +1, café visible, annulation, déconnexion', async ({ page }) => {
  await page.goto('/')
  await expect(page.getByRole('heading', { name: 'Connexion' })).toBeVisible()

  await page.getByLabel('Identifiant').fill(USER)
  await page.getByLabel('Mot de passe').fill('mauvais mot de passe')
  await page.getByRole('button', { name: 'Se connecter' }).click()
  await expect(page.getByRole('alert')).toHaveText('Identifiant ou mot de passe incorrect.')

  await page.getByLabel('Mot de passe').fill(PASSWORD)
  await page.getByRole('button', { name: 'Se connecter' }).click()
  await expect(page.getByRole('heading', { name: "Aujourd'hui" })).toBeVisible()
  await expect(count(page)).toHaveText('0')
  await expect(page.getByText("Aucun café noté aujourd'hui.")).toBeVisible()

  await add(page).click()
  // L'écran répond tout de suite, sans attendre le worker.
  await expect(count(page)).toHaveText('1')
  await expect(page.getByText('Aucun café noté')).toBeHidden()
  await expect(cancelButtons(page)).toHaveCount(1)

  // Le café vient bien du serveur : il survit à un rechargement.
  const request = page.context().request
  await waitForServerCount(request, cookie, today, 1)
  await page.reload()
  await expect(count(page)).toHaveText('1')
  const [cup] = await serverDay(request, cookie, today)
  expect(cup.source).toBe('webapp')
  await expect(page.getByText(cup.time, { exact: true })).toBeVisible()

  const cancelSent = page.waitForRequest((r) => r.url().endsWith('/v1/records'))
  await cancelButtons(page).click()
  const cancel = sentRecord(await cancelSent)
  expect(cancel.kind).toBe('record.cancel')
  expect(cancel.payload).toEqual({ source: 'webapp', key: cup.key })
  await expect(count(page)).toHaveText('0')
  await expect(cancelButtons(page)).toHaveCount(0)
  await expect(page.getByText("Aucun café noté aujourd'hui.")).toBeVisible()

  await waitForServerCount(request, cookie, today, 0)
  await page.reload()
  await expect(count(page)).toHaveText('0')

  await page.getByRole('button', { name: 'Se déconnecter' }).click()
  await expect(page.getByRole('heading', { name: 'Connexion' })).toBeVisible()
  await page.reload()
  await expect(page.getByRole('heading', { name: 'Connexion' })).toBeVisible()
})

test('le cookie de session porte tous ses attributs dans le navigateur', async ({ page, context }) => {
  await login(page)
  const session = (await context.cookies()).find((c) => c.name === 'lt_session')!
  expect(session.httpOnly).toBe(true)
  expect(session.secure).toBe(true)
  expect(session.sameSite).toBe('Strict')
  expect(await page.evaluate(() => document.cookie)).not.toContain('lt_session')
})

test('un double envoi du même clic ne compte qu’un café', async ({ page, context }) => {
  await login(page)
  const statuses: string[] = []
  const keys: string[] = []
  await page.route('**/v1/records', async (route) => {
    keys.push(sentRecord(route.request()).key)
    const first = await route.fetch()
    statuses.push((await first.json()).results[0].status)
    const second = await route.fetch()
    statuses.push((await second.json()).results[0].status)
    await route.fulfill({ response: second })
  })
  await add(page).click()
  await expect(count(page)).toHaveText('1')
  await expect.poll(() => statuses).toEqual(['accepted', 'duplicate'])
  expect(keys).toHaveLength(1)
  await waitForServerCount(context.request, cookie, today, 1)
  await page.reload()
  await expect(count(page)).toHaveText('1')
})

test('une réponse perdue est renvoyée avec la même clé, sans doublon', async ({ page, context }) => {
  await login(page)
  const keys: string[] = []
  await page.route('**/v1/records', async (route) => {
    keys.push(sentRecord(route.request()).key)
    if (keys.length === 1) {
      await route.fetch() // le serveur l'écrit, mais la réponse se perd
      await route.abort('connectionreset')
      return
    }
    await route.continue()
  })
  await add(page).click()
  await expect(count(page)).toHaveText('1')
  await expect.poll(() => keys.length).toBe(2)
  expect(keys[1]).toBe(keys[0])
  await expect(page.getByRole('alert')).toHaveCount(0)
  await waitForServerCount(context.request, cookie, today, 1)
  await page.reload()
  await expect(count(page)).toHaveText('1')
})

test('deux clics font deux cafés', async ({ page, context }) => {
  await login(page)
  await add(page).click()
  await add(page).click()
  await expect(count(page)).toHaveText('2')
  await waitForServerCount(context.request, cookie, today, 2)
  await expect(count(page)).toHaveText('2')
})

test('le nombre ne clignote pas entre l’écriture et le serveur', async ({ page, context }) => {
  await login(page)
  await expect(count(page)).toHaveText('0')
  await page.evaluate(() => {
    const seen: string[] = []
    ;(window as unknown as { seen: string[] }).seen = seen
    const el = document.querySelector('.count-value')!
    new MutationObserver(() => seen.push(el.textContent!)).observe(el, {
      characterData: true,
      childList: true,
      subtree: true,
    })
  })
  await add(page).click()
  await waitForServerCount(context.request, cookie, today, 1)
  // Laisse la relecture arriver, puis une annulation.
  await page.waitForTimeout(1500)
  await cancelButtons(page).click()
  await waitForServerCount(context.request, cookie, today, 0)
  await page.waitForTimeout(1500)
  const seen = await page.evaluate(() => (window as unknown as { seen: string[] }).seen)
  expect(seen).toEqual(['1', '0'])
})

test('un serveur en retard ne contredit pas l’écran, et l’attente se dit', async ({ page }) => {
  test.setTimeout(30_000)
  await login(page)
  // Le worker ne suit pas : le serveur continue de répondre « aucun café ».
  await page.route('**/v1/days/**', (route) => route.fulfill({ json: { date: today, count: 0, cups: [] } }))
  await add(page).click()
  // Le texte précis : l'alerte de la musique, selon les écoutes déjà reçues, porte le même rôle.
  const waiting = page.getByRole('status').filter({ hasText: "C'est bien enregistré" })
  await expect(waiting).toBeVisible({ timeout: 20_000 })
  await expect(count(page)).toHaveText('1')
  await page.unroute('**/v1/days/**')
  await expect(waiting).toHaveCount(0, { timeout: 10_000 })
  await expect(count(page)).toHaveText('1')
})

test('un café d’une autre source s’annule aussi', async ({ page, context, playwright }) => {
  const api = await playwright.request.newContext({ baseURL: API_URL })
  const occurredAt = new Date().toISOString().replace('Z', '+00:00')
  const sent = await api.post('/v1/records', {
    headers: { Authorization: `Bearer ${IOS_TOKEN}` },
    data: {
      records: [
        { key: crypto.randomUUID(), kind: 'coffee.cup', schema_version: 1, occurred_at: occurredAt, payload: {} },
      ],
    },
  })
  expect(sent.status()).toBe(200)
  await api.dispose()
  await waitForServerCount(context.request, cookie, today, 1)
  const [cup] = await serverDay(context.request, cookie, today)
  expect(cup.source).toBe('ios')

  await login(page)
  await expect(count(page)).toHaveText('1')
  const cancelSent = page.waitForRequest((r) => r.url().endsWith('/v1/records'))
  await cancelButtons(page).click()
  expect(sentRecord(await cancelSent).payload).toEqual({ source: 'ios', key: cup.key })
  await expect(count(page)).toHaveText('0')
  await waitForServerCount(context.request, cookie, today, 0)
})

test('une écriture en échec se voit, ne compte pas, et se renvoie avec la même clé', async ({ page, context }) => {
  await login(page)
  const keys: string[] = []
  await page.route('**/v1/records', (route) => {
    keys.push(sentRecord(route.request()).key)
    return route.abort('internetdisconnected')
  })
  await add(page).click()
  await expect(count(page)).toHaveText('1')
  await expect(page.getByRole('alert')).toContainText("n'a pas été enregistré", { timeout: 10_000 })
  await expect(count(page)).toHaveText('0')
  expect(new Set(keys).size).toBe(1)
  expect(keys.length).toBeGreaterThan(1)

  await page.unroute('**/v1/records')
  const retried = page.waitForRequest((r) => r.url().endsWith('/v1/records'))
  await page.getByRole('button', { name: 'Réessayer' }).click()
  expect(sentRecord(await retried).key).toBe(keys[0])
  await expect(count(page)).toHaveText('1')
  await expect(page.getByRole('alert')).toHaveCount(0)
  await waitForServerCount(context.request, cookie, today, 1)
})

test('une annulation en échec rend le café et se signale', async ({ page, context }) => {
  await login(page)
  await add(page).click()
  await waitForServerCount(context.request, cookie, today, 1)
  await page.route('**/v1/records', (route) => route.abort('internetdisconnected'))
  await cancelButtons(page).click()
  await expect(count(page)).toHaveText('0')
  await expect(page.getByRole('alert')).toContainText("n'a pas été enregistrée", { timeout: 10_000 })
  await expect(count(page)).toHaveText('1')
  await expect(cancelButtons(page)).toHaveCount(1)
  await page.getByRole('button', { name: 'Ignorer' }).click()
  await expect(page.getByRole('alert')).toHaveCount(0)
  await expect(count(page)).toHaveText('1')
})

test('une écriture refusée par le serveur se signale sans proposer de renvoi', async ({ page }) => {
  await login(page)
  await page.route('**/v1/records', (route) =>
    route.fulfill({
      json: { request_id: 'x', results: [{ key: 'k', status: 'rejected', reason: 'invalid_field', field: 'key' }] },
    }),
  )
  await add(page).click()
  await expect(page.getByRole('alert')).toContainText('refusé par le serveur')
  await expect(count(page)).toHaveText('0')
  await expect(page.getByRole('button', { name: 'Réessayer' })).toHaveCount(0)
})

test('une lecture en échec n’affiche jamais un zéro', async ({ page }) => {
  await login(page)
  await page.route('**/v1/days/**', (route) => route.abort('internetdisconnected'))
  await page.getByRole('button', { name: 'Jour précédent' }).click()
  await expect(page.getByRole('alert')).toContainText("Impossible d'actualiser")
  await expect(count(page)).toHaveText('–')
  await expect(page.getByText('Aucun café noté')).toBeHidden()
  await page.unroute('**/v1/days/**')
  // La musique a aussi échoué, et a son propre bouton.
  await page.getByRole('alert').getByRole('button', { name: 'Réessayer' }).click()
  await expect(count(page)).toHaveText('0')
  await expect(page.getByRole('alert')).toHaveCount(0)
})

test('naviguer : jour suivant bloqué aujourd’hui, heure choisie rattachée au jour affiché', async ({ page, context }) => {
  await login(page)
  await expect(page.getByRole('button', { name: 'Jour suivant' })).toBeDisabled()
  const yesterday = addDays(today, -1)

  await page.getByRole('button', { name: 'Jour précédent' }).click()
  await expect(page.getByRole('heading', { name: 'Hier' })).toBeVisible()
  await expect(page.getByRole('button', { name: 'Jour suivant' })).toBeEnabled()
  await expect(page.getByText('Aucun café noté hier.')).toBeVisible()

  // Pas de « maintenant » un autre jour : le + ouvre le choix de l'heure.
  await page.getByRole('button', { name: 'Ajouter un café à une autre heure' }).click()
  await page.getByLabel('Heure du café').fill('08:15')
  const sent = page.waitForRequest((r) => r.url().endsWith('/v1/records'))
  await page.getByRole('button', { name: 'Ajouter', exact: true }).click()
  const record = sentRecord(await sent)
  expect(record.occurred_at).toMatch(new RegExp(`^${yesterday}T08:15:00\\+0[12]:00$`))
  await expect(count(page)).toHaveText('1')
  await expect(page.getByText('08:15', { exact: true })).toBeVisible()

  await waitForServerCount(context.request, cookie, yesterday, 1)
  expect((await serverDay(context.request, cookie, yesterday))[0].time).toBe('08:15')

  await page.getByRole('button', { name: 'Jour suivant' }).click()
  await expect(page.getByRole('heading', { name: "Aujourd'hui" })).toBeVisible()
  await expect(count(page)).toHaveText('0')
  await expect(page.getByRole('button', { name: 'Jour suivant' })).toBeDisabled()
})

test('une heure future est refusée, sans rien envoyer', async ({ page }) => {
  // Le 23:59 d'aujourd'hui n'a de sens qu'avant cette minute.
  test.skip(new Intl.DateTimeFormat('fr-FR', { timeZone: 'Europe/Paris', timeStyle: 'short' }).format(new Date()) === '23:59')
  await login(page)
  let sent = 0
  await page.route('**/v1/records', (route) => {
    sent++
    return route.continue()
  })
  await page.getByRole('button', { name: 'Noter à une autre heure' }).click()
  await page.getByLabel('Heure du café').fill('23:59')
  await page.getByRole('button', { name: 'Ajouter', exact: true }).click()
  await expect(page.getByRole('alert')).toHaveText("Cette heure n'est pas encore passée.")
  await expect(count(page)).toHaveText('0')
  expect(sent).toBe(0)
})

test('le 25 oct. 2026 : 02:30 prend son premier passage, en heure d’été', async ({ page, context }) => {
  test.skip(today <= '2026-10-25', 'le jour doit être passé')
  await clearDay(context.request, cookie, '2026-10-25')
  await login(page)
  for (let date = today; date > '2026-10-25'; date = addDays(date, -1)) {
    await page.getByRole('button', { name: 'Jour précédent' }).click()
  }
  await expect(page.getByText('Dimanche 25 octobre')).toBeVisible()
  await page.getByRole('button', { name: 'Noter à une autre heure' }).click()
  await page.getByLabel('Heure du café').fill('02:30')
  const sent = page.waitForRequest((r) => r.url().endsWith('/v1/records'))
  await page.getByRole('button', { name: 'Ajouter', exact: true }).click()
  expect(sentRecord(await sent).occurred_at).toBe('2026-10-25T02:30:00+02:00')
  await waitForServerCount(context.request, cookie, '2026-10-25', 1)
  await expect(page.getByText('02:30', { exact: true })).toBeVisible()
  await clearDay(context.request, cookie, '2026-10-25')
})

test('une session expirée renvoie à la connexion sans perdre l’écriture', async ({ page, context }) => {
  await login(page)
  await context.clearCookies()
  await add(page).click()
  await expect(page.getByText('Session expirée. Reconnectez-vous.')).toBeVisible()

  await page.getByLabel('Identifiant').fill(USER)
  await page.getByLabel('Mot de passe').fill(PASSWORD)
  await page.getByRole('button', { name: 'Se connecter' }).click()
  await expect(page.getByRole('alert')).toContainText("n'a pas été enregistré")
  await page.getByRole('button', { name: 'Réessayer' }).click()
  await expect(count(page)).toHaveText('1')
  await waitForServerCount(context.request, cookie, today, 1)
})

test('la page se déclare installable', async ({ page, request }) => {
  await page.goto('/')
  await expect(page.locator('meta[name="viewport"]')).toHaveAttribute('content', /viewport-fit=cover/)
  const href = await page.locator('link[rel="manifest"]').getAttribute('href')
  const manifest = await (await request.get(href!)).json()
  expect(manifest.display).toBe('standalone')
  const sizes = manifest.icons.map((icon: { sizes: string }) => icon.sizes)
  expect(sizes).toEqual(expect.arrayContaining(['192x192', '512x512']))
  const touchIcon = await page.locator('link[rel="apple-touch-icon"]').getAttribute('href')
  for (const src of [...manifest.icons.map((icon: { src: string }) => icon.src), touchIcon]) {
    const response = await request.get(src)
    expect(response.status(), src).toBe(200)
    expect(response.headers()['content-type']).toBe('image/png')
  }
})
