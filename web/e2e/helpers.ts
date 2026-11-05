import { expect, type APIRequestContext, type Page } from '@playwright/test'

export const USER = process.env.E2E_USER!
export const PASSWORD = process.env.E2E_PASSWORD!
export const API_URL = process.env.E2E_API_URL!
export const IOS_TOKEN = process.env.E2E_IOS_TOKEN!
export const LASTFM_TOKEN = process.env.E2E_LASTFM_TOKEN!
export const APPLE_TOKEN = process.env.E2E_APPLE_TOKEN!
export const GARMIN_TOKEN = process.env.E2E_GARMIN_TOKEN!
export const YOUTUBE_TOKEN = process.env.E2E_YOUTUBE_TOKEN!

export function parisToday(): string {
  return new Intl.DateTimeFormat('en-CA', { timeZone: 'Europe/Paris' }).format(new Date())
}

export function addDays(date: string, days: number): string {
  const [y, m, d] = date.split('-').map(Number)
  return new Date(Date.UTC(y, m - 1, d + days)).toISOString().slice(0, 10)
}

export async function login(page: Page) {
  await page.goto('/')
  await page.getByLabel('Identifiant').fill(USER)
  await page.getByLabel('Mot de passe').fill(PASSWORD)
  await page.getByRole('button', { name: 'Se connecter' }).click()
  await expect(page.getByRole('button', { name: 'Ajouter un café' })).toBeVisible()
}

/** Le nombre affiché : celui du gros chiffre, pas le nombre de lignes. */
export const count = (page: Page) => page.getByTestId('count').locator('.count-value')

/** Une session d'API à part, pour préparer et vérifier sans passer par l'écran. */
export async function apiSession(request: APIRequestContext) {
  const response = await request.post('/v1/auth/login', {
    headers: { 'X-CSRF-Protection': '1' },
    data: { username: USER, password: PASSWORD },
  })
  expect(response.status()).toBe(204)
  return response
    .headersArray()
    .find((header) => header.name.toLowerCase() === 'set-cookie')!
    .value.split(';')[0]
}

export interface ServerCup {
  time: string
  source: string
  key: string
  occurred_at: string
}

export async function serverDay(request: APIRequestContext, cookie: string, date: string): Promise<ServerCup[]> {
  const response = await request.get(`/v1/days/${date}`, { headers: { Cookie: cookie } })
  expect(response.status()).toBe(200)
  return (await response.json()).cups
}

/** Vide une journée en l'annulant, comme le ferait l'app, et attend le worker. */
export async function clearDay(request: APIRequestContext, cookie: string, date: string) {
  const cups = await serverDay(request, cookie, date)
  if (cups.length > 0) {
    const response = await request.post('/v1/records', {
      headers: { Cookie: cookie, 'X-CSRF-Protection': '1' },
      data: {
        records: cups.map((cup) => ({
          key: crypto.randomUUID(),
          kind: 'record.cancel',
          schema_version: 1,
          occurred_at: new Date().toISOString().replace('Z', '+00:00'),
          payload: { source: cup.source, key: cup.key },
        })),
      },
    })
    expect(response.status()).toBe(200)
  }
  await expect.poll(async () => (await serverDay(request, cookie, date)).length, { timeout: 10_000 }).toBe(0)
}

export async function waitForServerCount(request: APIRequestContext, cookie: string, date: string, expected: number) {
  await expect.poll(async () => (await serverDay(request, cookie, date)).length, { timeout: 10_000 }).toBe(expected)
}

// Les écoutes entrent par la vraie API d'ingestion, avec le token de la source, comme le collecteur.
export const noon = (date: string, minute = 0) => `${date}T12:${String(minute).padStart(2, '0')}:00+01:00`

export interface Listen {
  artist: string
  track: string
  album?: string
}

export async function send(request: APIRequestContext, records: unknown[], token = LASTFM_TOKEN) {
  const response = await request.post(`${API_URL}/v1/records`, {
    headers: { Authorization: `Bearer ${token}` },
    data: { records },
  })
  expect(response.status()).toBe(200)
  const body = await response.json()
  for (const result of body.results) expect(['accepted', 'duplicate']).toContain(result.status)
}

export const scrobble = (date: string, minute: number, { artist, track, album = '' }: Listen) => ({
  key: `${date}-${minute}-${artist}-${track}`,
  kind: 'lastfm.scrobble',
  schema_version: 1,
  occurred_at: noon(date, minute),
  payload: {
    artist: { mbid: '', '#text': artist },
    album: { mbid: '', '#text': album },
    name: track,
    date: { uts: '0', '#text': '' },
  },
})

/** Attend que le worker ait tout interprété, sans supposer de sa vitesse. */
export async function waitForListens(request: APIRequestContext, cookie: string, date: string, expected: number) {
  await expect
    .poll(
      async () => {
        const response = await request.get(`/v1/days/${date}/music`, { headers: { Cookie: cookie } })
        return (await response.json()).listens
      },
      { timeout: 30_000 },
    )
    .toBe(expected)
}

export async function previousDay(page: Page, times: number) {
  for (let i = 0; i < times; i++) await page.getByRole('button', { name: 'Jour précédent' }).click()
}

export async function checkLayout(page: Page) {
  const problems = await page.evaluate(() => {
    const out: string[] = []
    if (document.documentElement.scrollWidth > window.innerWidth) out.push('défilement horizontal')
    for (const el of document.querySelectorAll('button, input, a')) {
      // Un élément caché (le tiroir fermé, un bouton propre à une largeur) n'est pas une cible.
      if (!el.checkVisibility({ visibilityProperty: true })) continue
      const { width, height, right } = el.getBoundingClientRect()
      const name = el.getAttribute('aria-label') ?? el.textContent ?? el.tagName
      if (height < 44 || width < 44) out.push(`${name} : ${width}×${height}`)
      if (right > window.innerWidth) out.push(`${name} dépasse à droite`)
    }
    return out
  })
  expect(problems).toEqual([])
}

/** Ouvre le tiroir s'il y en a un (iPhone) ; sur un écran large, la barre est déjà là. */
export async function openMenu(page: Page) {
  const button = page.getByRole('button', { name: 'Menu' })
  if (await button.isVisible()) {
    await button.click()
    await expect(page.getByRole('dialog', { name: 'Menu' })).toBeVisible()
  }
}

export async function logoutFromMenu(page: Page) {
  await openMenu(page)
  await page.getByRole('button', { name: 'Se déconnecter' }).click()
}

/** La largeur d'un Mac : la barre latérale fixe. */
export const DESKTOP = { width: 1280, height: 800 }

/**
 * Capture de toute la page. Sur Mac, la fenêtre prend d'abord la hauteur de la page : la barre latérale
 * est collée à la fenêtre (100dvh), et une capture « pleine page » la couperait au milieu.
 */
export async function fullShot(page: Page, path: string) {
  const viewport = page.viewportSize()
  if (!viewport || viewport.width < 900) {
    await page.screenshot({ path, fullPage: true })
    return
  }
  const height = await page.evaluate(() => document.documentElement.scrollHeight)
  await page.setViewportSize({ width: viewport.width, height: Math.max(viewport.height, height) })
  await page.screenshot({ path })
  await page.setViewportSize(viewport)
}
