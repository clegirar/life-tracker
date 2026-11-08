import { execFileSync } from 'node:child_process'
import { expect, test, type APIRequestContext, type Page } from '@playwright/test'
import { addDays, API_URL, apiSession, checkLayout, DESKTOP, fullShot, GITHUB_TOKEN, login, openMenu, parisToday, send } from './helpers'

// GitHub : la carte du jour, les cases du mois, la carte de la page d'état et la fiche Sources. Les faits
// entrent par la vraie API d'ingestion avec le token de la source `github` (web/test.sh), comme le
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

interface Repo {
  id: string
  name: string
}

const MINE: Repo = { id: 'R_kgDOFICTIF1', name: 'moi/depot-fictif' }
const ORG: Repo = { id: 'R_kgDOFICTIF2', name: 'orga-fictive/autre-depot' }
const NEW: Repo = { id: 'R_kgDOFICTIF3', name: 'moi/nouveau-depot-fictif' }

let shas = 0
/** Un commit, comme le collecteur : sha fictif, lignes facultatives (un très gros commit n'en a pas). */
function commit(repo: Repo, date: string, time: string, title: string, lines: [number, number] | null) {
  const n = ++shas
  const oid = Array.from({ length: 40 }, (_, i) => ((n * 7_919 + i * i * 104_729) % 16).toString(16)).join('')
  return {
    key: `commit:${oid}`,
    kind: 'github.commit',
    schema_version: 1,
    occurred_at: paris(date, time),
    payload: {
      repo,
      oid,
      authored_at: paris(date, time),
      committed_at: paris(date, time),
      title,
      additions: lines?.[0] ?? null,
      deletions: lines?.[1] ?? null,
      url: `https://github.com/${repo.name}/commit/${oid}`,
    },
  }
}

const pullRequest = (repo: Repo, action: 'opened' | 'merged', date: string, time: string, number: number, title: string, lines?: [number, number]) => ({
  key: `pr:PR_kwDOFICTIF${number}:${action}`,
  kind: 'github.pull_request',
  schema_version: 1,
  occurred_at: paris(date, time),
  payload: {
    id: `PR_kwDOFICTIF${number}`,
    action,
    repo,
    number,
    title,
    url: `https://github.com/${repo.name}/pull/${number}`,
    ...(lines ? { additions: lines[0], deletions: lines[1] } : {}),
  },
})

const COMMIT_TITLE = 'Ajoute le compteur fictif'

async function seed(request: APIRequestContext) {
  await send(
    request,
    [
      // Vingt jours avant : les premiers commits sur les deux dépôts, leurs découvertes.
      commit(MINE, BEFORE, '20:00', 'Premier commit fictif', [10, 0]),
      commit(ORG, BEFORE, '20:30', 'Premier commit fictif ailleurs', [5, 0]),
      // Le jour : trois commits, dont un très gros sans lignes ; une pull request ouverte puis fusionnée.
      commit(MINE, DATE, '09:12', 'Corrige la date fictive', [3, 1]),
      commit(MINE, DATE, '21:15', COMMIT_TITLE, [40, 2]),
      commit(MINE, DATE, '22:40', 'Importe les données fictives', null),
      pullRequest(MINE, 'opened', DATE, '21:30', 12, 'Compteur fictif'),
      pullRequest(MINE, 'merged', DATE, '22:00', 12, 'Compteur fictif', [43, 3]),
      // Sur le dépôt d'une organisation : une review, une issue et un commentaire, sans commit.
      {
        key: 'review:PRR_kwDOFICTIF1',
        kind: 'github.review',
        schema_version: 1,
        occurred_at: paris(DATE, '11:05'),
        payload: { id: 'PRR_kwDOFICTIF1', state: 'APPROVED', repo: ORG, number: 3, title: "La PR fictive d'un autre", url: `https://github.com/${ORG.name}/pull/3#pullrequestreview-1` },
      },
      {
        key: 'issue:I_kwDOFICTIF1',
        kind: 'github.issue',
        schema_version: 1,
        occurred_at: paris(DATE, '10:00'),
        payload: { id: 'I_kwDOFICTIF1', repo: ORG, number: 4, title: 'Un bogue fictif', url: `https://github.com/${ORG.name}/issues/4` },
      },
      {
        key: 'comment:IC_kwDOFICTIF1',
        kind: 'github.comment',
        schema_version: 1,
        occurred_at: paris(DATE, '10:30'),
        payload: { id: 'IC_kwDOFICTIF1', repo: ORG, target: 'issue', number: 4, title: 'Un bogue fictif', url: `https://github.com/${ORG.name}/issues/4#issuecomment-1` },
      },
      // Le lendemain : un premier commit sur un nouveau dépôt, une découverte.
      commit(NEW, NEXT, '10:20', 'Démarre le dépôt fictif', [120, 0]),
    ],
    GITHUB_TOKEN,
  )
  const cookie = await apiSession(request)
  await expect
    .poll(
      async () => {
        const body = await (await request.get(`/v1/days/${DATE}/github`, { headers: { Cookie: cookie } })).json()
        return [body.commits, body.pull_requests, body.merged, body.reviews, body.issues, body.comments, body.repos.length]
      },
      { timeout: 30_000 },
    )
    .toEqual([3, 1, 1, 1, 1, 1, 2])
}

async function report(request: APIRequestContext, status: Record<string, unknown>) {
  const response = await request.put(`${API_URL}/v1/sources/me/status`, {
    headers: { Authorization: `Bearer ${GITHUB_TOKEN}` },
    data: status,
  })
  expect(response.status()).toBe(204)
}

async function heartbeat(request: APIRequestContext) {
  const response = await request.post(`${API_URL}/v1/heartbeat`, { headers: { Authorization: `Bearer ${GITHUB_TOKEN}` } })
  expect(response.status()).toBe(200)
}

const silence = () => sql("UPDATE core.sources SET last_seen_at = now() - interval '8 hours' WHERE name = 'github'")
const card = (page: Page) => page.getByRole('article', { name: 'GitHub' })
const inDays = (days: number) => new Date(Date.now() + days * 86_400_000).toISOString()

/** Seulement GitHub dans les filtres, pour le mois et les captures : après la connexion, qui attend le café. */
async function onlyGithub(page: Page) {
  await page.evaluate(() => window.localStorage.setItem('calendar.metrics', JSON.stringify(['code.commit', 'code.pull_request'])))
}

async function openStatus(page: Page) {
  await openMenu(page)
  await page.getByRole('link', { name: /^État/ }).click()
  await expect(page.getByRole('heading', { level: 1, name: 'État' })).toBeVisible()
  await expect(page.getByRole('dialog', { name: 'Menu' })).toBeHidden()
}

async function openSources(page: Page) {
  await openMenu(page)
  await page.getByRole('link', { name: 'Sources' }).click()
  await expect(page.getByRole('heading', { level: 1, name: 'Sources' })).toBeVisible()
  await expect(page.getByRole('dialog', { name: 'Menu' })).toBeHidden()
}

test.beforeAll(async ({ request }) => {
  await seed(request)
})

// Les autres fichiers retrouvent une source sans état ni silence.
test.afterEach(async ({ request }) => {
  sql("UPDATE core.sources SET status = NULL, status_reported_at = NULL WHERE name = 'github'")
  await heartbeat(request)
})

test('la carte du jour : commits, lignes, dépôts, pull requests, reviews, issues et commentaires', async ({ page, request }) => {
  await heartbeat(request)
  await login(page)
  await page.goto(`/jour/${DATE}`)
  const github = page.getByTestId('github')
  await expect(github.getByRole('heading', { name: 'GitHub', level: 2 })).toBeVisible()
  await expect(github.getByTestId('github-commits')).toHaveText('3')
  // Les lignes des commits qui les donnent ; le très gros commit est dit à part, jamais compté zéro.
  await expect(github.getByTestId('github-lines')).toHaveText('+43−3')
  await expect(github).toContainText('43 lignes ajoutées, 3 supprimées')
  await expect(github.getByTestId('github-lines-unknown')).toHaveText('Lignes inconnues pour 1 commit.')
  await expect(github.getByTestId('github-summary')).toHaveText('1 pull request ouverte · 1 fusionnée · 1 review · 1 issue · 1 commentaire')

  // Par commits décroissants : le dépôt aux commits, puis celui de l'organisation.
  const repos = github.getByTestId('github-repo')
  await expect(repos).toHaveCount(2)
  await expect(repos.nth(0).getByRole('heading', { level: 3 })).toHaveText(MINE.name)
  await expect(repos.nth(0).getByRole('link', { name: MINE.name })).toHaveAttribute('href', `https://github.com/${MINE.name}`)
  await expect(repos.nth(0)).toContainText('3 commits · +43 −3 · 1 pull request ouverte · 1 fusionnée')
  const mine = repos.nth(0).getByTestId('github-row')
  await expect(mine).toHaveCount(5)
  await expect(mine.nth(0)).toHaveAttribute('data-kind', 'commit')
  await expect(mine.nth(0)).toContainText('Corrige la date fictive')
  await expect(mine.nth(0)).toContainText('09:12 ·')
  await expect(mine.nth(0).getByTestId('github-row-lines')).toHaveText('+3 −1')
  await expect(mine.nth(1)).toContainText(COMMIT_TITLE)
  await expect(mine.nth(1).getByTestId('github-row-lines')).toHaveText('+40 −2')
  await expect(mine.nth(2)).toContainText('Importe les données fictives')
  await expect(mine.nth(2)).toContainText('lignes inconnues')
  await expect(mine.nth(3)).toHaveAttribute('data-kind', 'pull_request')
  await expect(mine.nth(3)).toContainText('21:30 · Pull request #12 ouverte')
  await expect(mine.nth(3).getByTestId('github-row-lines')).toHaveCount(0)
  await expect(mine.nth(4)).toHaveAttribute('data-kind', 'merged')
  await expect(mine.nth(4)).toContainText('22:00 · Pull request #12 fusionnée')
  await expect(mine.nth(4).getByTestId('github-row-lines')).toHaveText('+43 −3')

  const link = mine.nth(3).getByRole('link')
  await expect(link).toHaveAttribute('href', `https://github.com/${MINE.name}/pull/12`)
  await expect(link).toHaveAttribute('target', '_blank')
  await expect(link).toHaveAttribute('rel', 'noopener noreferrer')

  await expect(repos.nth(1).getByRole('heading', { level: 3 })).toHaveText(ORG.name)
  await expect(repos.nth(1)).toContainText('1 review · 1 issue · 1 commentaire')
  await expect(repos.nth(1).getByTestId('github-row')).toHaveText([
    /La PR fictive d'un autre11:05 · Review approuvée #3/,
    /Un bogue fictif10:00 · Issue #4 ouverte/,
    /Un bogue fictif10:30 · Commentaire sur l'issue #4/,
  ])
  await expect(repos.nth(1).getByTestId('github-row').nth(2).getByRole('link')).toHaveAttribute('href', `https://github.com/${ORG.name}/issues/4#issuecomment-1`)
  await expect(github.locator('img')).toHaveCount(0)
  await checkLayout(page)

  // Des dépôts déjà commités avant : aucune découverte, et pas de section du tout.
  await expect(github.getByTestId('github-discoveries')).toHaveCount(0)
  await expect(github).not.toContainText('Découvertes')

  // Le lendemain : un nouveau dépôt, une découverte, et pas de résumé sans autre activité.
  await page.goto(`/jour/${NEXT}`)
  await expect(page.getByTestId('github-commits')).toHaveText('1')
  await expect(page.getByTestId('github-lines')).toHaveText('+120−0')
  await expect(page.getByTestId('github-lines-unknown')).toHaveCount(0)
  await expect(page.getByTestId('github-summary')).toHaveCount(0)
  const found = page.getByTestId('github-discoveries')
  await expect(found.getByRole('heading', { name: /Découvertes/ })).toContainText('1')
  await expect(found.getByTestId('github-discovery')).toHaveText([/^moi\/nouveau-depot-fictif/])
  await expect(found.getByRole('link')).toHaveAttribute('href', `https://github.com/${NEW.name}`)
  await checkLayout(page)

  // Avant la première donnée : pas de données, pas « rien sur GitHub ».
  await page.goto(`/jour/${addDays(BEFORE, -1)}`)
  await expect(page.getByTestId('github')).toContainText('Pas de données GitHub à cette date.')
  // Entre deux jours de code, le collecteur ayant donné signe de vie depuis : rien sur GitHub.
  await page.goto(`/jour/${addDays(DATE, -1)}`)
  await expect(page.getByTestId('github')).toContainText('Rien sur GitHub ce jour-là.')
})

test('les cases du mois : une pastille « GitHub », commits et pull requests en points', async ({ page }) => {
  await login(page)
  // Une métrique du groupe gardée suffit : la pastille coche les deux, comme « Activité ».
  await page.evaluate(() => window.localStorage.setItem('calendar.metrics', JSON.stringify(['code.commit'])))
  await page.goto(`/mois/${DATE.slice(0, 7)}`)
  const cell = page.locator(`.cell[data-date="${DATE}"]`)
  for (const metric of ['code.commit', 'code.pull_request']) {
    await expect(cell.locator(`.day-dot[data-metric="${metric}"]`)).toHaveCount(1)
    await expect(cell.locator(`.day-dot[data-metric="${metric}"]`)).toHaveClass(/heat-github/)
  }
  await expect(cell).toHaveAccessibleName(/3 commits.*1 pull request\b/)

  // Une seule pastille « GitHub », après les jeux, cochée ; la décocher retire les deux.
  const filters = page.getByRole('group', { name: /Métriques|Filtres/ })
  const labels = await filters.getByRole('button').allTextContents()
  expect(labels.indexOf('GitHub')).toBeGreaterThan(labels.indexOf('Jeux'))
  expect(labels).not.toContain('Commits')
  expect(labels).not.toContain('Pull requests')
  const chip = filters.getByRole('button', { name: 'GitHub' })
  await expect(chip).toHaveAttribute('aria-pressed', 'true')
  await expect(chip).toHaveClass(/heat-github/)
  await chip.click()
  await expect(chip).toHaveAttribute('aria-pressed', 'false')
  await expect(cell.locator('.day-dot')).toHaveCount(0)
})

test("un collecteur muet : l'alerte du jour, et la carte d'état silencieuse", async ({ page, request }) => {
  silence()
  await login(page)
  await page.goto(`/jour/${DATE}`)
  await expect(page.getByTestId('github-alert')).toContainText('Le collecteur GitHub ne donne plus signe de vie.')

  await openStatus(page)
  const github = card(page)
  await expect(github.getByTestId('status-word')).toHaveText('Silencieux')
  await expect(github).toContainText('Aucun contact réussi avec GitHub depuis plus de 6 h : des données peuvent manquer.')
  await checkLayout(page)

  await heartbeat(request)
  await page.reload()
  await expect(github.getByTestId('status-word')).toHaveText('Fonctionne')
})

test("la page d'état : dernier commit, dernière activité, échéance du token, puis SSO, token refusé et limite", async ({ page, request }) => {
  await heartbeat(request)
  await report(request, { state: 'ok', expires_at: inDays(200), last_collected_at: new Date(Date.now() - 20 * 60_000).toISOString(), next_run_at: new Date(Date.now() + 10 * 60_000).toISOString() })
  await login(page)
  await openStatus(page)
  const github = card(page)
  await expect(github.getByTestId('status-word')).toHaveText('Fonctionne')
  await expect(github).toContainText("Dernière passeà l'instant")
  await expect(github).toContainText('Dernière collecteil y a 20 min')
  await expect(github).toContainText(/Tokenvalable jusqu'au/)
  await expect(github).toContainText(/Dernier commit(le \d+\s\S+|hier) à/)
  await expect(github).toContainText(/Dernière activité(le \d+\s\S+|hier) à/)
  // Une ligne pour le commit et une pour le reste, pas une par métrique.
  await expect(github).not.toContainText('Dernière donnée')

  // Quatorze jours avant l'échéance : en orange, et à faire.
  await report(request, { state: 'ok', expires_at: inDays(10) })
  await page.reload()
  await expect(github.getByTestId('status-word')).toHaveText('À faire')
  await expect(github).toContainText(/Token\s*expire le .*, dans \d+ jours/)
  await expect(github).toContainText('GITHUB_TOKEN_EXPIRES_ON')

  await report(request, { state: 'action_required', reason: 'sso_required', expires_at: inDays(200) })
  await page.reload()
  await expect(github.getByTestId('status-word')).toHaveText('À faire')
  await expect(github).toContainText('Une organisation à authentification unique (SAML) refuse le token')
  await expect(github).toContainText('« Configure SSO »')
  await expect(page.getByTestId('menu-dot')).toBeVisible()
  await checkLayout(page)

  await report(request, { state: 'action_required', reason: 'key_rejected' })
  await page.reload()
  await expect(github).toContainText("GitHub refuse le token d'accès personnel")
  await expect(github).toContainText('GITHUB_API_TOKEN')

  await report(request, { state: 'error', reason: 'rate_limited' })
  await page.reload()
  await expect(github.getByTestId('status-word')).toHaveText('En panne')
  await expect(github).toContainText('5 000 points par heure')
})

test('la fiche Sources : le collecteur, le token et son échéance vivante, le token de la source', async ({ page, request }) => {
  await heartbeat(request)
  await report(request, { state: 'ok', expires_at: inDays(10) })
  await login(page)
  await openSources(page)
  const sheet = page.locator('article[data-tracker="github"]')
  await expect(sheet.getByRole('heading', { level: 3 })).toHaveText('GitHub')
  await expect(sheet).toContainText("l'API GraphQL de GitHub tous les quarts d'heure")
  await expect(sheet).toContainText('GITHUB_API_TOKEN')
  await expect(sheet).toContainText('GITHUB_INGEST_TOKEN')
  const expiry = sheet.getByRole('region', { name: /^Token d'accès personnel GitHub/ }).getByTestId('key-expiry')
  await expect(expiry).toContainText(/Expire le .*, dans \d+ jours\.$/)
  await expect(expiry).toHaveAttribute('data-warn', 'soon')
  await checkLayout(page)
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

        test(`GitHub à ${width} px, ${colorScheme}`, async ({ page, request }) => {
          const shot = (name: string) => fullShot(page, `${SHOTS}/${width}-${colorScheme}-${name}.png`)
          await heartbeat(request)
          await report(request, { state: 'ok', expires_at: inDays(200), last_collected_at: new Date(Date.now() - 5 * 60_000).toISOString(), next_run_at: new Date(Date.now() + 10 * 60_000).toISOString() })
          await login(page)
          await onlyGithub(page)
          await page.goto(`/jour/${DATE}`)
          await expect(page.getByTestId('github-commits')).toHaveText('3')
          await checkLayout(page)
          await shot('github-jour')

          await page.goto(`/jour/${NEXT}`)
          await expect(page.getByTestId('github-discovery')).toHaveCount(1)
          await checkLayout(page)
          await shot('github-jour-decouverte')

          await page.goto(`/mois/${DATE.slice(0, 7)}`)
          await expect(page.locator(`.cell[data-date="${DATE}"] .day-dot[data-metric="code.commit"]`)).toHaveCount(1)
          await page.waitForTimeout(300) // la fin de l'arrivée
          await checkLayout(page)
          await shot('github-mois')

          await openStatus(page)
          await expect(card(page).getByTestId('status-word')).toHaveText('Fonctionne')
          await checkLayout(page)
          await shot('github-etat')

          // Le token à dix jours de son échéance : la ligne en orange.
          await report(request, { state: 'ok', expires_at: inDays(10) })
          await page.reload()
          await expect(card(page).getByTestId('status-word')).toHaveText('À faire')
          await checkLayout(page)
          await shot('github-etat-echeance')

          await report(request, { state: 'action_required', reason: 'sso_required', expires_at: inDays(200) })
          await page.reload()
          await expect(card(page)).toContainText('« Configure SSO »')
          await checkLayout(page)
          await shot('github-etat-sso')

          await report(request, { state: 'ok', expires_at: inDays(10) })
          await openSources(page)
          // La page reprend l'état déjà lu : relue, elle montre l'échéance proche.
          await page.reload()
          await expect(page.locator('article[data-tracker="github"]').getByTestId('key-expiry').first()).toHaveAttribute('data-warn', 'soon')
          await page.locator('article[data-tracker="github"]').scrollIntoViewIfNeeded()
          await checkLayout(page)
          await shot('github-sources')
        })
      })
    }
  })
}
