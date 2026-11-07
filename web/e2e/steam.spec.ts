import { execFileSync } from 'node:child_process'
import { expect, test, type APIRequestContext, type Page } from '@playwright/test'
import { addDays, API_URL, apiSession, checkLayout, DESKTOP, fullShot, login, openMenu, parisToday, send, STEAM_TOKEN } from './helpers'

// Steam : la carte du jour, la case du mois et la carte de la page d'état. Les relevés et les succès
// entrent par la vraie API d'ingestion avec le token de la source `steam` (web/test.sh), comme le
// collecteur du NAS ; son état aussi, par `PUT /v1/sources/me/status`. Seul le silence se simule dans
// la base jetable. Données fictives.
const SHOTS = process.env.E2E_SCREENSHOTS ?? 'e2e-artifacts/screenshots'
const DATE = addDays(parisToday(), -12)
const NEXT = addDays(DATE, 1)
const BEFORE = addDays(DATE, -20)

function sql(statement: string) {
  execFileSync('docker', ['exec', process.env.E2E_DB_CONTAINER!, 'psql', '-q', '-U', 'postgres', '-d', 'tracker', '-c', statement])
}

/** L'instant d'une heure de Paris, avec son décalage du jour (heure d'été ou d'hiver). */
function paris(date: string, time: string): string {
  for (const offset of ['+01:00', '+02:00']) {
    const at = `${date}T${time}:00${offset}`
    const local = new Intl.DateTimeFormat('sv-SE', { timeZone: 'Europe/Paris', dateStyle: 'short', timeStyle: 'short' }).format(new Date(at))
    if (local === `${date} ${time}`) return at
  }
  throw new Error(`heure introuvable : ${date} ${time}`)
}

const epoch = (date: string, time: string) => Math.floor(Date.parse(paris(date, time)) / 1000)

interface Previous {
  minutes: number
  date: string
  time: string
}

/** Un relevé du compteur d'un jeu, avec le relevé précédent, comme le collecteur. */
const reading = (appid: number, name: string, date: string, time: string, minutes: number, lastPlayed: [string, string] | null, previous: Previous | null) => ({
  key: `playtime:${appid}:${minutes}`,
  kind: 'steam.playtime',
  schema_version: 1,
  occurred_at: paris(date, time),
  payload: {
    appid,
    name,
    playtime_forever: minutes,
    playtime_disconnected: 0,
    rtime_last_played: lastPlayed && epoch(...lastPlayed),
    previous: previous && { playtime_forever: previous.minutes, observed_at: paris(previous.date, previous.time) },
  },
})

const achievement = (appid: number, gameName: string, apiname: string, name: string, date: string, time: string) => ({
  key: `achievement:${appid}:${apiname}`,
  kind: 'steam.achievement',
  schema_version: 1,
  occurred_at: paris(date, time),
  payload: { appid, game_name: gameName, apiname, name },
})

const GAME = 'Jeu Fictif'
const OTHER = 'Autre Jeu Fictif'
const SILENT = 'Jeu Fictif Sans Partie'
const NEW = 'Nouveau Jeu Fictif'

async function seed(request: APIRequestContext) {
  await send(
    request,
    [
      // Vingt jours avant : les points de départ, puis une demi-heure de jeu.
      reading(4242, GAME, BEFORE, '20:00', 9_000, null, null),
      reading(777, OTHER, BEFORE, '20:00', 100, null, null),
      reading(4242, GAME, BEFORE, '21:00', 9_030, [BEFORE, '20:55'], { minutes: 9_000, date: BEFORE, time: '20:00' }),
      // 40 min, finies à 21:55 : de 21:15 à 21:55.
      reading(4242, GAME, DATE, '22:00', 9_070, [DATE, '21:55'], { minutes: 9_030, date: BEFORE, time: '21:00' }),
      // 30 min à cheval sur minuit : 15 min pour chaque jour.
      reading(777, OTHER, NEXT, '00:20', 130, [NEXT, '00:15'], { minutes: 100, date: BEFORE, time: '20:00' }),
      achievement(4242, GAME, 'ACH_FICTIF', 'Succès fictif', DATE, '21:50'),
      // Un succès sans partie reçue ce jour-là : le jeu est listé, sans temps.
      achievement(99, SILENT, 'ACH_HORS_LIGNE', 'Succès hors ligne fictif', DATE, '08:02'),
      // Acheté le lendemain, après le début du suivi : parti de zéro, 20 min de 10:00 à 10:20, une découverte.
      reading(555, NEW, NEXT, '10:30', 20, [NEXT, '10:20'], { minutes: 0, date: NEXT, time: '09:45' }),
    ],
    STEAM_TOKEN,
  )
  const cookie = await apiSession(request)
  await expect
    .poll(
      async () => {
        const body = await (await request.get(`/v1/days/${DATE}/games`, { headers: { Cookie: cookie } })).json()
        return [body.minutes, body.achievements, body.games.length]
      },
      { timeout: 30_000 },
    )
    .toEqual([55, 2, 3])
}

async function report(request: APIRequestContext, status: Record<string, unknown>) {
  const response = await request.put(`${API_URL}/v1/sources/me/status`, {
    headers: { Authorization: `Bearer ${STEAM_TOKEN}` },
    data: status,
  })
  expect(response.status()).toBe(204)
}

async function heartbeat(request: APIRequestContext) {
  const response = await request.post(`${API_URL}/v1/heartbeat`, { headers: { Authorization: `Bearer ${STEAM_TOKEN}` } })
  expect(response.status()).toBe(200)
}

const silence = () => sql("UPDATE core.sources SET last_seen_at = now() - interval '8 hours' WHERE name = 'steam'")
const card = (page: Page) => page.getByRole('article', { name: 'Steam' })

/** Seulement Steam dans les filtres (les succès n'en ont pas), pour le mois et les captures : après la connexion, qui attend le café. */
async function onlySteam(page: Page) {
  await page.evaluate(() => window.localStorage.setItem('calendar.metrics', JSON.stringify(['game.play'])))
}

async function openStatus(page: Page) {
  await openMenu(page)
  await page.getByRole('link', { name: /^État/ }).click()
  await expect(page.getByRole('heading', { level: 1, name: 'État' })).toBeVisible()
  await expect(page.getByRole('dialog', { name: 'Menu' })).toBeHidden()
}

test.beforeAll(async ({ request }) => {
  await seed(request)
})

// Les autres fichiers retrouvent une source sans état ni silence.
test.afterEach(async ({ request }) => {
  sql("UPDATE core.sources SET status = NULL, status_reported_at = NULL WHERE name = 'steam'")
  await heartbeat(request)
})

test('la carte du jour : temps, jeux, heures, total, succès et minuit', async ({ page, request }) => {
  await heartbeat(request)
  await login(page)
  await page.goto(`/jour/${DATE}`)
  const steam = page.getByTestId('steam')
  await expect(steam.getByRole('heading', { name: 'Steam' })).toBeVisible()
  // 40 min de Jeu Fictif, et le quart d'heure d'avant minuit de l'autre.
  await expect(steam.getByTestId('steam-minutes')).toHaveText('55 min')
  await expect(steam.getByTestId('steam-achievement-count')).toHaveText('2')

  const rows = steam.getByTestId('steam-game')
  await expect(rows).toHaveCount(3)
  // Par temps, puis le jeu qui n'a qu'un succès.
  await expect(rows.nth(0)).toContainText(GAME)
  await expect(rows.nth(0)).toContainText('21:15 – 21:55 · 151 h au total')
  await expect(rows.nth(0)).toContainText('40 min')
  await expect(rows.nth(0).getByTestId('steam-achievement')).toHaveText(['Succès fictif21:50'])
  // Finie le lendemain : ses vraies heures, et le total relevé à la fin de ce jour.
  await expect(rows.nth(1)).toContainText(OTHER)
  await expect(rows.nth(1)).toContainText('23:45 – 00:15 · 1 h 40 au total')
  await expect(rows.nth(1)).toContainText('15 min')
  await expect(rows.nth(1).getByTestId('steam-achievement')).toHaveCount(0)
  await expect(rows.nth(2)).toContainText(SILENT)
  await expect(rows.nth(2).locator('.row-count')).toHaveCount(0)
  await expect(rows.nth(2).getByTestId('steam-achievement')).toHaveText(['Succès hors ligne fictif08:02'])

  const link = rows.nth(0).getByRole('link')
  await expect(link).toHaveAttribute('href', 'https://store.steampowered.com/app/4242/')
  await expect(link).toHaveAttribute('target', '_blank')
  await expect(link).toHaveAttribute('rel', 'noopener noreferrer')
  await expect(steam.locator('img')).toHaveCount(0)
  await checkLayout(page)

  // Des jeux déjà joués avant le suivi : aucune découverte, et pas de section du tout.
  await expect(steam.getByTestId('steam-discoveries')).toHaveCount(0)
  await expect(steam).not.toContainText('Découvertes')
  await expect(steam).not.toContainText(/aucune découverte/i)

  // Le lendemain : le jeu acheté entre-temps, une découverte ; le quart d'heure d'après minuit, et le nouveau total.
  await page.goto(`/jour/${NEXT}`)
  await expect(page.getByTestId('steam-minutes')).toHaveText('35 min')
  await expect(page.getByTestId('steam-achievement-count')).toHaveCount(0)
  await expect(page.getByTestId('steam-game')).toHaveText([/10:00 – 10:20 · 20 min au total/, /23:45 – 00:15 · 2 h 10 au total/])
  const found = page.getByTestId('steam-discoveries')
  await expect(found.getByRole('heading', { name: /Découvertes/ })).toContainText('1')
  await expect(found.getByTestId('steam-discovery')).toHaveText([NEW])
  await expect(found.getByRole('link')).toHaveAttribute('href', 'https://store.steampowered.com/app/555/')
  await checkLayout(page)

  // Avant le premier relevé : pas de données, pas « pas de jeu ».
  await page.goto(`/jour/${addDays(BEFORE, -1)}`)
  await expect(page.getByTestId('steam')).toContainText('Pas de données Steam à cette date.')
  // Entre deux parties, le collecteur ayant donné signe de vie depuis : pas de jeu.
  await page.goto(`/jour/${addDays(DATE, -1)}`)
  await expect(page.getByTestId('steam')).toContainText('Pas de jeu ce jour-là.')
})

test('la case du mois : le temps du jour, dans la couleur de Steam', async ({ page }) => {
  await login(page)
  await page.evaluate(() => window.localStorage.setItem('calendar.metrics', JSON.stringify(['game.play'])))
  await page.goto(`/mois/${DATE.slice(0, 7)}`)
  const cell = page.locator(`.cell[data-date="${DATE}"]`)
  await expect(cell.locator('.cell-value[data-metric="game.play"]')).toHaveText('55')
  await expect(cell).toHaveAccessibleName(/55 min/)
  await expect(cell.locator('.cell-value')).toHaveClass(/heat-steam/)
})

test("un collecteur muet : l'alerte du jour, l'attente, et la carte d'état silencieuse", async ({ page, request }) => {
  silence()
  await login(page)
  await page.goto(`/jour/${DATE}`)
  await expect(page.getByTestId('steam-alert')).toContainText('Le collecteur Steam ne donne plus signe de vie.')
  await page.goto(`/jour/${parisToday()}`)
  // Le dernier contact date de 8 h : aujourd'hui ou hier, on ne sait pas encore pour aujourd'hui s'il était hier.
  await expect(page.getByTestId('steam')).toContainText(/Pas encore de données pour ce jour|Pas de jeu aujourd'hui\./)

  await openStatus(page)
  const steam = card(page)
  await expect(steam.getByTestId('status-word')).toHaveText('Silencieux')
  await expect(steam).toContainText('Aucun contact réussi avec Steam depuis plus de 6 h : des données peuvent manquer.')
  await expect(steam).toContainText('sudo docker compose ps collector')
  await checkLayout(page)

  await heartbeat(request)
  await page.reload()
  await expect(steam.getByTestId('status-word')).toHaveText('Fonctionne')
})

test("la page d'état : dernière passe, dernière partie, dernier succès, puis le profil privé et la clé refusée", async ({ page, request }) => {
  await heartbeat(request)
  await report(request, { state: 'ok', last_collected_at: new Date(Date.now() - 20 * 60_000).toISOString(), next_run_at: new Date(Date.now() + 10 * 60_000).toISOString() })
  await login(page)
  await openStatus(page)
  const steam = card(page)
  await expect(steam.getByTestId('status-word')).toHaveText('Fonctionne')
  await expect(steam).toContainText("Dernière passeà l'instant")
  await expect(steam).toContainText('Dernière collecteil y a 20 min')
  await expect(steam).toContainText(/Dernière partie(le \d+\s\S+|hier) à/)
  await expect(steam).toContainText(/Dernier succès(le \d+\s\S+|hier) à/)
  // Une clé Steam n'expire pas : pas de ligne d'échéance.
  await expect(steam).not.toContainText('Clé')

  await report(request, { state: 'action_required', reason: 'profile_private', last_collected_at: null })
  await page.reload()
  await expect(steam.getByTestId('status-word')).toHaveText('À faire')
  await expect(steam).toContainText('Le profil Steam est privé : Steam ne donne ni les jeux ni les succès.')
  await expect(steam).toContainText('« Détails des jeux »')
  await expect(page.getByTestId('menu-dot')).toBeVisible()
  await checkLayout(page)

  await report(request, { state: 'action_required', reason: 'key_rejected' })
  await page.reload()
  await expect(steam).toContainText('Steam refuse la clé du collecteur')
  await expect(steam).toContainText('STEAM_API_KEY')

  await report(request, { state: 'error', reason: 'rate_limited' })
  await page.reload()
  await expect(steam.getByTestId('status-word')).toHaveText('En panne')
  await expect(steam).toContainText("Le service limite les appels pour l'instant.")
})

for (const colorScheme of ['light', 'dark'] as const) {
  test.describe(colorScheme, () => {
    test.use({ colorScheme })

    for (const [width, viewport] of [
      [390, null],
      [1280, DESKTOP],
    ] as const) {
      test.describe(`${width} px`, () => {
        if (viewport) test.use({ viewport, isMobile: false, hasTouch: false })

        test(`Steam à ${width} px, ${colorScheme}`, async ({ page, request }) => {
          const shot = (name: string) => fullShot(page, `${SHOTS}/${width}-${colorScheme}-${name}.png`)
          await heartbeat(request)
          await report(request, { state: 'ok', last_collected_at: new Date(Date.now() - 5 * 60_000).toISOString(), next_run_at: new Date(Date.now() + 10 * 60_000).toISOString() })
          await login(page)
          await onlySteam(page)
          await page.goto(`/jour/${DATE}`)
          await expect(page.getByTestId('steam-minutes')).toHaveText('55 min')
          await checkLayout(page)
          await shot('steam-jour')

          await page.goto(`/jour/${NEXT}`)
          await expect(page.getByTestId('steam-discovery')).toHaveText([NEW])
          await checkLayout(page)
          await shot('steam-jour-decouverte')

          await page.goto(`/mois/${DATE.slice(0, 7)}`)
          await expect(page.locator(`.cell[data-date="${DATE}"] .cell-value[data-metric="game.play"]`)).toHaveText('55')
          await page.waitForTimeout(300) // la fin de l'arrivée
          await checkLayout(page)
          await shot('steam-mois')

          await openStatus(page)
          await expect(card(page).getByTestId('status-word')).toHaveText('Fonctionne')
          await checkLayout(page)
          await shot('steam-etat')

          await report(request, { state: 'action_required', reason: 'profile_private', last_collected_at: null })
          await page.reload()
          await expect(card(page).getByTestId('status-word')).toHaveText('À faire')
          await checkLayout(page)
          await shot('steam-etat-profil-prive')

          await openMenu(page)
          await page.getByRole('link', { name: 'Sources' }).click()
          await expect(page.getByRole('heading', { level: 1, name: 'Sources' })).toBeVisible()
          await expect(page.getByRole('dialog', { name: 'Menu' })).toBeHidden()
          await page.locator('article[data-tracker="steam"]').scrollIntoViewIfNeeded()
          await checkLayout(page)
          await shot('steam-sources')
        })
      })
    }
  })
}
