import { execFileSync } from 'node:child_process'
import { expect, test, type APIRequestContext, type Page } from '@playwright/test'
import { addDays, API_URL, apiSession, checkLayout, DESKTOP, fullShot, login, openMenu, parisToday, RIOT_TOKEN, send, STEAM_TOKEN, VALORANT_TOKEN } from './helpers'

// Riot (LoL, TFT) et VALORANT (HenrikDev) : la section « Riot » de la carte « Jeux », à côté de celle de
// Steam, la case du mois (le temps seul : les parties n'ont ni case ni pastille) et les deux cartes de la
// page d'état. Les parties entrent par la vraie API d'ingestion avec les tokens des
// sources `riot` et `valorant` (web/test.sh), telles que les envoient les collecteurs ; leur état aussi,
// par `PUT /v1/sources/me/status`. Données fictives : champion, agent et carte inventés.
const SHOTS = process.env.E2E_SCREENSHOTS ?? 'e2e-artifacts/screenshots'
const DATE = addDays(parisToday(), -9)
// Un jour sans partie, entre deux soirs de jeu : « pas de jeu », pas « pas de données ».
const QUIET = addDays(DATE, -1)
// Un soir de LoL seul : la section « Riot », sans celle de Steam.
const EARLIER = addDays(DATE, -5)
const STEAM_GAME = 'Troisième Jeu Fictif'

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

/** « 14:35 » : `minutes` après 14:00. */
const clock = (minutes: number) => `${String(14 + Math.floor(minutes / 60)).padStart(2, '0')}:${String(minutes % 60).padStart(2, '0')}`

/** Un relevé du compteur de Steam, avec le précédent, comme `collector::steam`. */
const steamReading = (date: string, time: string, minutes: number, lastPlayed: [string, string] | null, previous: { minutes: number; date: string; time: string } | null) => ({
  key: `playtime:8888:${minutes}`,
  kind: 'steam.playtime',
  schema_version: 1,
  occurred_at: paris(date, time),
  payload: {
    appid: 8888,
    name: STEAM_GAME,
    playtime_forever: minutes,
    playtime_disconnected: 0,
    rtime_last_played: lastPlayed && Math.floor(Date.parse(paris(...lastPlayed)) / 1000),
    previous: previous && { playtime_forever: previous.minutes, observed_at: paris(previous.date, previous.time) },
  },
})

const CHAMPION = 'Champion Fictif'
const AGENT = 'Agent Fictif'
const MAP = 'Carte Fictive'

/** Une partie de LoL, comme `collector::riot` : ma ligne seulement. */
const lol = (id: number, date: string, time: string, durationS: number, result: 'win' | 'loss' | 'remake') => ({
  key: `lol:EUW1_${id}`,
  kind: 'riot.lol_match',
  schema_version: 1,
  occurred_at: paris(date, time),
  payload: {
    match_id: `EUW1_${id}`,
    started_at: paris(date, time),
    ended_at: null,
    duration_s: durationS,
    queue_id: 420,
    game_mode: 'CLASSIC',
    end_of_game_result: 'GameComplete',
    champion: CHAMPION,
    win: result === 'win',
    position: 'MIDDLE',
    remake: result === 'remake',
  },
})

/** Une partie de TFT : le début, mon temps jusqu'à mon élimination, ma place. */
const tft = (id: number, time: string, eliminatedS: number, placement: number) => ({
  key: `tft:EUW1_${id}`,
  kind: 'riot.tft_match',
  schema_version: 1,
  occurred_at: paris(DATE, time),
  payload: {
    match_id: `EUW1_${id}`,
    started_at: paris(DATE, time),
    ended_at: null,
    game_length_s: 2400,
    eliminated_s: eliminatedS,
    queue_id: 1160,
    game_type: 'pairs',
    set_number: 15,
    placement,
    partner_group_id: 1,
  },
})

/** Une partie de VALORANT, comme `collector::valorant` (HenrikDev). */
const valorant = (id: string, time: string, minutes: number, won: boolean, rounds: [number, number]) => ({
  key: `valorant:${id}`,
  kind: 'valorant.match',
  schema_version: 1,
  occurred_at: paris(DATE, time),
  payload: {
    match_id: id,
    started_at: paris(DATE, time),
    duration_ms: minutes * 60_000,
    queue_id: 'competitive',
    queue_name: 'Competitive',
    map: MAP,
    agent: AGENT,
    team: 'Red',
    won,
    rounds_won: rounds[0],
    rounds_lost: rounds[1],
  },
})

// Sept parties de LoL toutes les 35 min depuis 14:00 (une remake de 3 min 30), trois de TFT, deux de
// VALORANT : douze parties, un soir chargé.
const LOL_RESULTS = ['win', 'win', 'loss', 'win', 'remake', 'loss', 'win'] as const

async function seed(request: APIRequestContext) {
  await send(
    request,
    [
      lol(1000, EARLIER, '20:00', 1500, 'loss'),
      ...LOL_RESULTS.map((result, index) => lol(1001 + index, DATE, clock(index * 35), result === 'remake' ? 210 : 1800, result)),
      tft(2001, '18:15', 1500, 1),
      tft(2002, '18:55', 2100, 4),
      tft(2003, '19:40', 1200, 7),
    ],
    RIOT_TOKEN,
  )
  await send(
    request,
    [
      valorant('0f0f0f0f-0000-4000-8000-000000000001', '21:00', 40, true, [13, 8]),
      valorant('0f0f0f0f-0000-4000-8000-000000000002', '21:50', 45, false, [12, 12]),
    ],
    VALORANT_TOKEN,
  )
  // Steam le même soir : 45 min de 22:05 à 22:50, d'un point de départ relevé cinq jours plus tôt.
  await send(
    request,
    [
      steamReading(EARLIER, '12:00', 300, null, null),
      steamReading(DATE, '23:00', 345, [DATE, '22:50'], { minutes: 300, date: EARLIER, time: '12:00' }),
    ],
    STEAM_TOKEN,
  )
  const cookie = await apiSession(request)
  await expect
    .poll(
      async () => {
        const body = await (await request.get(`/v1/days/${DATE}/games`, { headers: { Cookie: cookie } })).json()
        return [body.matches.length, body.games.length]
      },
      { timeout: 30_000 },
    )
    .toEqual([12, 4])
}

async function report(request: APIRequestContext, token: string, status: Record<string, unknown>) {
  const response = await request.put(`${API_URL}/v1/sources/me/status`, {
    headers: { Authorization: `Bearer ${token}` },
    data: status,
  })
  expect(response.status()).toBe(204)
}

async function heartbeat(request: APIRequestContext, token: string) {
  const response = await request.post(`${API_URL}/v1/heartbeat`, { headers: { Authorization: `Bearer ${token}` } })
  expect(response.status()).toBe(200)
}

const card = (page: Page, title: string) => page.getByRole('article', { name: title })
const RIOT = 'Riot Games (LoL, TFT)'
const VALORANT = 'Valorant (HenrikDev)'

/** Seulement « Jeux » dans les filtres, pour les captures. */
async function onlyGames(page: Page) {
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

// Les autres fichiers retrouvent des sources sans état ni silence.
test.afterEach(async ({ request }) => {
  sql("UPDATE core.sources SET status = NULL, status_reported_at = NULL WHERE name IN ('riot', 'valorant')")
  await heartbeat(request, RIOT_TOKEN)
  await heartbeat(request, VALORANT_TOKEN)
})

test('la carte du jour : Steam puis Riot, les parties sous leur jeu, avec le bilan, et un soir chargé replié', async ({ page, request }) => {
  await heartbeat(request, RIOT_TOKEN)
  await login(page)
  await page.goto(`/jour/${DATE}`)
  const games = page.getByTestId('games')
  // Un seul titre « Jeux », puis les deux sections, Steam d'abord.
  await expect(games.getByRole('heading', { level: 2 })).toHaveText('Jeux')
  await expect(games.getByRole('heading', { level: 3 })).toHaveText(['Steam', 'Riot'])
  await expect(games.getByTestId('games-match-count')).toHaveText('12')
  await expect(games.getByTestId('games-steam').getByTestId('game').locator('.row-name')).toHaveText([STEAM_GAME])

  // Par temps : LoL, puis VALORANT, puis TFT ; aucun lien vers un magasin.
  const rows = games.getByTestId('games-riot').getByTestId('game')
  await expect(rows.locator('.row-name')).toHaveText(['League of Legends', 'VALORANT', 'Teamfight Tactics'])
  await expect(games.getByTestId('games-riot').getByRole('link')).toHaveCount(0)

  const lolRow = rows.nth(0)
  // Des espaces insécables : « 1 remake » ne se coupe pas.
  await expect(lolRow.locator('.row-detail')).toHaveText('14:00 – 18:00 · 7\u00a0parties · 4\u00a0V\u00a0– 2\u00a0D · 1\u00a0remake')
  // Cinq parties d'emblée, puis le reste à la demande.
  const lolMatches = lolRow.getByTestId('game-match')
  await expect(lolMatches).toHaveCount(5)
  await expect(lolMatches.nth(0)).toHaveText(`14:00Victoire${CHAMPION} · MilieuClassée solo/duo30 min`)
  await expect(lolMatches.nth(2).locator('.game-match-result')).toHaveText('Défaite')
  await expect(lolMatches.nth(4).locator('.game-match-result')).toHaveText('Remake')
  await checkLayout(page)
  await lolRow.getByRole('button', { name: '2 autres parties' }).click()
  await expect(lolMatches).toHaveCount(7)
  await expect(lolRow.getByRole('button', { name: /autres parties/ })).toHaveCount(0)

  // VALORANT : l'agent, la carte, la file et les rounds ; une égalité.
  const valorantRow = rows.nth(1)
  await expect(valorantRow.locator('.row-detail')).toHaveText('21:00 – 22:35 · 2 parties · 1 V – 0 D – 1 N')
  await expect(valorantRow.getByTestId('game-match')).toHaveText([
    `21:00Victoire${AGENT} · ${MAP}Competitive · 13–840 min`,
    `21:50Égalité${AGENT} · ${MAP}Competitive · 12–1245 min`,
  ])

  // TFT : la place, et mon temps jusqu'à mon élimination.
  const tftRow = rows.nth(2)
  await expect(tftRow.locator('.row-detail')).toContainText('3 parties · Top 4 : 2 sur 3')
  await expect(tftRow.locator('.game-match-result')).toHaveText(['1re', '4e', '7e'])
  await expect(tftRow.getByTestId('game-match').nth(0)).toHaveText('18:151reDouble Up25 min')

  // Les parties ne déplient que ce jour-là.
  await page.goto(`/jour/${addDays(DATE, -2)}`)
  await page.goto(`/jour/${DATE}`)
  await expect(page.getByTestId('games-riot').getByTestId('game').nth(0).getByTestId('game-match')).toHaveCount(5)
  await checkLayout(page)

  // Le jour d'avant, entre deux soirs de jeu : pas de jeu, et pas « pas de données ».
  await page.goto(`/jour/${QUIET}`)
  await expect(page.getByTestId('games')).toContainText(/Pas de jeu ce jour-là\.|Pas de jeu hier\./)
  await expect(page.getByTestId('game-match')).toHaveCount(0)
  // Plus tôt, une seule partie et rien sur Steam : la section « Riot » seule, ni repli ni bouton.
  await page.goto(`/jour/${EARLIER}`)
  await expect(page.getByTestId('games').getByRole('heading', { level: 3 })).toHaveText(['Riot'])
  await expect(page.getByTestId('games-steam')).toHaveCount(0)
  await expect(page.getByTestId('game-match')).toHaveText([`20:00Défaite${CHAMPION} · MilieuClassée solo/duo25 min`])
  await expect(page.getByTestId('game').locator('.row-detail')).toHaveText('20:00 – 20:25 · 1\u00a0partie · 0\u00a0V\u00a0– 1\u00a0D')
})

test('plier « Riot » : le résumé reste, et le pli vaut pour les autres jours et la visite suivante', async ({ page }) => {
  await login(page)
  await page.goto(`/jour/${DATE}`)
  const riot = page.getByTestId('games-riot')
  const toggle = riot.getByRole('button', { name: /^Riot/ })
  await expect(toggle).toHaveAttribute('aria-expanded', 'true')
  await expect(toggle).toHaveAttribute('aria-controls', 'jeux-riot-contenu')
  // Ouverte : pas de résumé, la liste dit tout.
  await expect(riot.getByTestId('games-section-summary')).toHaveCount(0)
  await toggle.click()
  await expect(toggle).toHaveAttribute('aria-expanded', 'false')
  // 3 h 04 de LoL, 1 h 25 de VALORANT, 1 h 20 de TFT.
  await expect(riot.getByTestId('games-section-summary')).toHaveText('5 h 49 · 12\u00a0parties')
  await expect(page.locator('#jeux-riot-contenu')).toHaveAttribute('inert', '')
  // Repliée en hauteur : le contenu ne prend plus de place.
  await expect.poll(async () => (await page.locator('#jeux-riot-contenu').boundingBox())?.height).toBe(0)
  // Steam reste ouverte, chaque section a son pli.
  await expect(page.getByTestId('games-steam').getByRole('button', { name: /^Steam/ })).toHaveAttribute('aria-expanded', 'true')
  await expect(page.getByTestId('games-steam').getByTestId('game')).toBeVisible()
  await checkLayout(page)

  // Un autre jour, puis une nouvelle visite : toujours pliée.
  await page.goto(`/jour/${EARLIER}`)
  await expect(page.getByTestId('games-riot').getByRole('button', { name: /^Riot/ })).toHaveAttribute('aria-expanded', 'false')
  await expect(page.getByTestId('games-riot').getByTestId('games-section-summary')).toHaveText('25 min · 1\u00a0partie')
  await page.reload()
  const again = page.getByTestId('games-riot').getByRole('button', { name: /^Riot/ })
  await expect(again).toHaveAttribute('aria-expanded', 'false')

  // Au clavier : Entrée la rouvre.
  await again.focus()
  await page.keyboard.press('Enter')
  await expect(again).toHaveAttribute('aria-expanded', 'true')
  await expect(page.getByTestId('games-riot').getByTestId('game-match')).toBeVisible()
})

test('le mois : un seul filtre « Jeux », le temps de Steam et de Riot ensemble, sans pastille « Parties »', async ({ page }) => {
  await login(page)
  await onlyGames(page)
  await page.goto(`/mois/${DATE.slice(0, 7)}`)
  const filters = page.getByRole('group', { name: 'Métriques affichées' })
  await expect(filters.getByRole('button', { name: 'Jeux', exact: true })).toHaveAttribute('aria-pressed', 'true')
  await expect(filters.getByRole('button', { name: 'Parties', exact: true })).toHaveCount(0)
  const cell = page.locator(`.cell[data-date="${DATE}"]`)
  // 3 h 03 de LoL (une remake de 3 min 30), 1 h 25 de VALORANT, 1 h 20 de TFT et 45 min de Steam.
  await expect(cell.locator('.cell-value[data-metric="game.play"]')).toHaveText(/^6(,\d)? h$/)
  await expect(cell.locator('[data-metric="game.match"]')).toHaveCount(0)
  await expect(cell).not.toHaveAccessibleName(/parties/)
})

test("la page d'état : dernière partie, puis une clé Riot refusée et HenrikDev en panne", async ({ page, request }) => {
  await heartbeat(request, RIOT_TOKEN)
  await heartbeat(request, VALORANT_TOKEN)
  await report(request, RIOT_TOKEN, { state: 'ok', last_collected_at: new Date(Date.now() - 10 * 60_000).toISOString(), next_run_at: new Date(Date.now() + 5 * 60_000).toISOString() })
  await login(page)
  await openStatus(page)
  const riot = card(page, RIOT)
  await expect(riot.getByTestId('status-word')).toHaveText('Fonctionne')
  await expect(riot).toContainText("Dernière passeà l'instant")
  await expect(riot).toContainText('Dernière collecteil y a 10 min')
  // Une seule « Dernière partie » (le temps et la partie disent la même chose), et pas d'échéance.
  await expect(riot.getByText('Dernière partie')).toHaveCount(1)
  await expect(riot).not.toContainText('Échéance')
  await expect(card(page, VALORANT)).toContainText(/Dernière partie(le \d+\s\S+|hier) à/)

  await report(request, RIOT_TOKEN, { state: 'action_required', reason: 'key_rejected' })
  await report(request, VALORANT_TOKEN, { state: 'error', reason: 'service_unavailable' })
  await page.reload()
  await expect(riot.getByTestId('status-word')).toHaveText('À faire')
  await expect(riot).toContainText('Riot refuse une clé du collecteur')
  await expect(riot).toContainText('developer.riotgames.com')
  await expect(riot).toContainText('RIOT_LOL_API_KEY')
  await expect(riot).toContainText('RIOT_TFT_API_KEY')
  const henrik = card(page, VALORANT)
  await expect(henrik.getByTestId('status-word')).toHaveText('En panne')
  await expect(henrik).toContainText('HenrikDev ne répond pas, ou refuse la demande.')
  await expect(henrik).toContainText('non officielle')
  await expect(page.getByTestId('menu-dot')).toBeVisible()
  await checkLayout(page)

  await report(request, VALORANT_TOKEN, { state: 'action_required', reason: 'key_rejected' })
  await page.reload()
  await expect(henrik).toContainText('Discord de HenrikDev')
  await expect(henrik).toContainText('HENRIK_API_KEY')

  await report(request, RIOT_TOKEN, { state: 'error', reason: 'rate_limited' })
  await page.reload()
  await expect(riot.getByTestId('status-word')).toHaveText('En panne')
  await expect(riot).toContainText('100 par 2 min')
})

test('les fiches Sources : comment les parties arrivent, les clés et les tokens', async ({ page }) => {
  await login(page)
  await openMenu(page)
  await page.getByRole('link', { name: 'Sources' }).click()
  await expect(page.getByRole('heading', { level: 1, name: 'Sources' })).toBeVisible()
  const riot = page.locator('article[data-tracker="riot"]')
  await expect(riot).toContainText("l'API officielle de Riot")
  for (const name of ['RIOT_LOL_API_KEY', 'RIOT_TFT_API_KEY', 'RIOT_TOKEN']) await expect(riot).toContainText(name)
  const henrik = page.locator('article[data-tracker="valorant"]')
  await expect(henrik).toContainText('non officielle')
  for (const name of ['HENRIK_API_KEY', 'VALORANT_TOKEN']) await expect(henrik).toContainText(name)
  const html = await page.content()
  for (const secret of [RIOT_TOKEN, VALORANT_TOKEN]) expect(html.includes(secret)).toBe(false)
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

        test(`Riot à ${width} px, ${colorScheme}`, async ({ page, request }) => {
          const shot = (name: string) => fullShot(page, `${SHOTS}/${width}-${colorScheme}-${name}.png`)
          await heartbeat(request, RIOT_TOKEN)
          await heartbeat(request, VALORANT_TOKEN)
          await login(page)
          await onlyGames(page)
          await page.goto(`/jour/${DATE}`)
          await expect(page.getByTestId('games-match-count')).toHaveText('12')
          await checkLayout(page)
          await shot('riot-jour-steam-riot')

          // Riot pliée : le résumé à la place de la liste, Steam ouverte.
          await page.getByTestId('games-riot').getByRole('button', { name: /^Riot/ }).click()
          await expect(page.getByTestId('games-riot').getByTestId('games-section-summary')).toBeVisible()
          await page.waitForTimeout(300) // la fin du pli
          await checkLayout(page)
          await shot('riot-jour-riot-plie')
          await page.getByTestId('games-riot').getByRole('button', { name: /^Riot/ }).click()
          await expect(page.getByTestId('games-riot').getByRole('button', { name: /^Riot/ })).toHaveAttribute('aria-expanded', 'true')
          await page.waitForTimeout(300)

          await page.getByRole('button', { name: '2 autres parties' }).click()
          await expect(page.getByTestId('games-riot').getByTestId('game').nth(0).getByTestId('game-match')).toHaveCount(7)
          await checkLayout(page)
          await shot('riot-jour-deplie')

          await page.goto(`/jour/${EARLIER}`)
          await expect(page.getByTestId('games-riot').getByTestId('game-match')).toHaveCount(1)
          await checkLayout(page)
          await shot('riot-jour-riot-seul')

          await page.goto(`/jour/${QUIET}`)
          await expect(page.getByTestId('games')).toContainText(/Pas de jeu/)
          await checkLayout(page)
          await shot('riot-jour-sans-partie')

          await page.goto(`/mois/${DATE.slice(0, 7)}`)
          await expect(page.locator(`.cell[data-date="${DATE}"] .cell-value[data-metric="game.play"]`)).toHaveText(/^6(,\d)? h$/)
          await page.waitForTimeout(300) // la fin de l'arrivée
          await checkLayout(page)
          await shot('riot-mois')

          await report(request, RIOT_TOKEN, { state: 'action_required', reason: 'key_rejected' })
          await report(request, VALORANT_TOKEN, { state: 'error', reason: 'service_unavailable' })
          await openStatus(page)
          await expect(card(page, RIOT).getByTestId('status-word')).toHaveText('À faire')
          await card(page, RIOT).scrollIntoViewIfNeeded()
          await checkLayout(page)
          await shot('riot-etat-alerte')

          await openMenu(page)
          await page.getByRole('link', { name: 'Sources' }).click()
          await expect(page.getByRole('heading', { level: 1, name: 'Sources' })).toBeVisible()
          await expect(page.getByRole('dialog', { name: 'Menu' })).toBeHidden()
          await page.locator('article[data-tracker="riot"]').scrollIntoViewIfNeeded()
          await checkLayout(page)
          await shot('riot-sources')
        })
      })
    }
  })
}
