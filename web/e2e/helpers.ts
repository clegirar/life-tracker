import { expect, type APIRequestContext, type Page } from '@playwright/test'

export const USER = process.env.E2E_USER!
export const PASSWORD = process.env.E2E_PASSWORD!
export const API_URL = process.env.E2E_API_URL!
export const IOS_TOKEN = process.env.E2E_IOS_TOKEN!

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
