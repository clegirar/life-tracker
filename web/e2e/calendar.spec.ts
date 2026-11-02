import { execFileSync } from 'node:child_process'
import { expect, test, type APIRequestContext, type Page } from '@playwright/test'
import {
  addDays,
  API_URL,
  apiSession,
  checkLayout,
  clearDay,
  IOS_TOKEN,
  LASTFM_TOKEN,
  login,
  noon,
  parisToday,
  scrobble,
  send,
  serverDay,
  waitForListens,
  waitForServerCount,
} from './helpers'

// La vue du mois et la complétude. Des jours que les autres fichiers n'utilisent pas : 6 à 9, 17 à 20.
// Noms fictifs.
const SHOTS = process.env.E2E_SCREENSHOTS ?? 'e2e-artifacts/screenshots'
const today = parisToday()
const monthOf = (date: string) => date.slice(0, 7)
const monthsBetween = (from: string, to: string) => {
  const [fy, fm] = from.split('-').map(Number)
  const [ty, tm] = to.split('-').map(Number)
  return (fy - ty) * 12 + (fm - tm)
}

/** Seulement pour simuler une panne : la base jetable de web/test.sh, en superutilisateur. */
function sql(statement: string) {
  execFileSync('docker', ['exec', process.env.E2E_DB_CONTAINER!, 'psql', '-q', '-U', 'postgres', '-d', 'tracker', '-c', statement])
}

async function heartbeat(request: APIRequestContext) {
  const response = await request.post(`${API_URL}/v1/heartbeat`, { headers: { Authorization: `Bearer ${LASTFM_TOKEN}` } })
  expect(response.status()).toBe(200)
}

interface ServerDay {
  date: string
  values: Record<string, number | null>
  errors: number
  pending: string[]
}

async function serverMonthDay(request: APIRequestContext, cookie: string, date: string): Promise<ServerDay> {
  const response = await request.get(`/v1/months/${monthOf(date)}`, { headers: { Cookie: cookie } })
  expect(response.status()).toBe(200)
  expect(response.headers()['cache-control']).toBe('no-store')
  return (await response.json()).days.find((day: ServerDay) => day.date === date)
}

/** Depuis la page du jour d'aujourd'hui : ouvre le mois, puis recule jusqu'à celui de `date`. */
async function openMonthOf(page: Page, date: string) {
  await page.getByRole('button', { name: /^Voir le mois/ }).click()
  await expect(page.getByTestId('month-grid')).toBeVisible()
  for (let i = 0; i < monthsBetween(monthOf(today), monthOf(date)); i++) {
    await page.getByRole('button', { name: 'Mois précédent' }).click()
  }
  return cell(page, date)
}

const cell = (page: Page, date: string) => page.locator(`.cell[data-date="${date}"]`)

const broken = (date: string) => ({
  key: `${date}-cassé`,
  kind: 'lastfm.scrobble',
  schema_version: 1,
  occurred_at: noon(date, 45),
  payload: { unexpected: true },
})

test('un enregistrement en erreur marque sa journée, et le compteur global le compte', async ({ page, request }) => {
  const date = addDays(today, -6)
  const cookie = await apiSession(request)
  await send(request, [broken(date), scrobble(date, 0, { artist: 'Groupe du Mois', track: 'Titre du mois' })])
  await waitForListens(request, cookie, date, 1)
  await expect.poll(async () => (await serverMonthDay(request, cookie, date)).errors, { timeout: 30_000 }).toBe(1)

  await login(page)
  const day = await openMonthOf(page, date)
  await expect(day).toHaveAttribute('data-marker', 'incomplete')
  await expect(day).toHaveAccessibleName(/journée incomplète, 1 enregistrement en erreur/)
  await expect(page.getByTestId('error-count')).toContainText(/\d+ enregistrements? en erreur/)

  // La page du jour le dit aussi.
  await day.click()
  await expect(page.getByTestId('day-incomplete')).toContainText('Journée incomplète.')
  await expect(page.getByTestId('listens')).toHaveText('1')
})

test('une source muette marque les journées non couvertes, jusqu’à son rattrapage', async ({ page, request }) => {
  const cookie = await apiSession(request)
  try {
    sql("UPDATE core.sources SET last_seen_at = now() - interval '2 days' WHERE name = 'lastfm'")
    const since = addDays(today, -2)
    await expect.poll(async () => (await serverMonthDay(request, cookie, today)).pending).toEqual(['lastfm'])
    expect((await serverMonthDay(request, cookie, since)).pending).toEqual(['lastfm'])
    expect((await serverMonthDay(request, cookie, addDays(today, -4))).pending).toEqual([])
    expect((await serverMonthDay(request, cookie, addDays(today, 1))).pending).toEqual([])

    await login(page)
    const now = await openMonthOf(page, today)
    await expect(now).toHaveAccessibleName(/en attente de Last\.fm/)
    await expect(page.getByText('Last.fm ne répond plus.')).toBeVisible()
    await expect(page.locator('.legend')).toContainText('En attente d’une source')
  } finally {
    await heartbeat(request)
  }
  await page.reload()
  const now = await openMonthOf(page, today)
  await expect(now).not.toHaveAccessibleName(/en attente/)
  expect((await serverMonthDay(request, cookie, today)).pending).toEqual([])
})

test('un jour passé n’est jamais gardé : une donnée tardive le change', async ({ page, request }) => {
  const date = addDays(today, -7)
  const cookie = await apiSession(request)
  await clearDay(request, cookie, date)
  await login(page)
  const day = await openMonthOf(page, date)
  const before = (await serverMonthDay(request, cookie, date)).values.coffee
  await expect(day).toHaveAccessibleName(before === null ? /cafés : pas de données/ : new RegExp(`: ${before} cafés?,`))

  // Un café noté après coup par le raccourci, pendant que le mois est à l'écran.
  const response = await request.post(`${API_URL}/v1/records`, {
    headers: { Authorization: `Bearer ${IOS_TOKEN}` },
    data: { records: [{ key: crypto.randomUUID(), kind: 'coffee.cup', schema_version: 1, occurred_at: noon(date), payload: {} }] },
  })
  expect(response.status()).toBe(200)
  await waitForServerCount(request, cookie, date, 1)

  // Retour au jour puis au mois : relu, pas resservi.
  await day.click()
  await expect(page.getByTestId('count').locator('.count-value')).toHaveText('1')
  await page.getByRole('button', { name: /^Voir le mois/ }).click()
  await expect(cell(page, date).locator('.v-coffee')).toHaveText('1')
  expect(await serverDay(request, cookie, date)).toHaveLength(1)
  await clearDay(request, cookie, date)
})

for (const colorScheme of ['light', 'dark'] as const) {
  test.describe(colorScheme, () => {
    test.use({ colorScheme })

    test(`mois à 390 px, ${colorScheme}`, async ({ page, request }) => {
      const cookie = await apiSession(request)
      // Quelques jours d'écoutes et un trou de deux jours : de quoi lire la grille.
      for (const [daysAgo, listens] of [[8, 3], [9, 12], [17, 140], [18, 7], [20, 1]] as const) {
        const date = addDays(today, -daysAgo)
        await send(
          request,
          Array.from({ length: listens }, (_, i) => scrobble(date, i % 60, { artist: 'Groupe du Mois', track: `Titre ${i}` })).map(
            (record, i) => ({ ...record, key: `${record.key}-${i}`, occurred_at: `${date}T${String(10 + Math.floor(i / 60)).padStart(2, '0')}:${String(i % 60).padStart(2, '0')}:00+01:00` }),
          ),
        )
        await waitForListens(request, cookie, date, listens)
      }
      const shot = (name: string) => page.screenshot({ path: `${SHOTS}/${colorScheme}-mois-${name}.png`, fullPage: true })

      await login(page)
      await expect(page.getByRole('button', { name: /^Voir le mois/ })).toBeVisible()
      expect(page.viewportSize()!.width).toBe(390)
      await checkLayout(page)
      await shot('0-jour')
      await openMonthOf(page, today)
      await expect(cell(page, today)).toHaveAttribute('aria-current', 'date')
      await expect(cell(page, today)).toHaveAccessibleName(/écoute/)
      await page.waitForTimeout(300) // la fin de l'arrivée
      await checkLayout(page)
      await shot('1-mois')

      const busy = addDays(today, -17)
      for (let i = 0; i < monthsBetween(monthOf(today), monthOf(busy)); i++) {
        await page.getByRole('button', { name: 'Mois précédent' }).click()
      }
      await expect(cell(page, busy).locator('.v-music')).toHaveText('140')
      await checkLayout(page)
      await shot('2-mois-precedent')

      try {
        sql("UPDATE core.sources SET last_seen_at = now() - interval '2 days' WHERE name = 'lastfm'")
        await page.reload()
        await openMonthOf(page, today)
        await expect(page.getByText('Last.fm ne répond plus.')).toBeVisible()
        await checkLayout(page)
        await shot('3-en-attente')
      } finally {
        await heartbeat(request)
      }
    })
  })
}
