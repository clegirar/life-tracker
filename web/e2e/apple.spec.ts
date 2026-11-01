import { expect, test } from '@playwright/test'
import { addDays, APPLE_TOKEN, apiSession, checkLayout, login, noon, parisToday, previousDay, send, waitForListens } from './helpers'

// Une ligne de l'export telle que l'outil d'import l'envoie. Tout est fictif.
const SHOTS = process.env.E2E_SCREENSHOTS ?? 'e2e-artifacts/screenshots'
const today = parisToday()
const DAYS_AGO = 23
const date = addDays(today, -DAYS_AGO)

const row = (minute: number, song: string, artist: string | null) => ({
  key: `play_end:${date}:${minute}`,
  kind: 'apple.play_history_row',
  schema_version: 1,
  occurred_at: noon(date, minute),
  payload: {
    song,
    album: 'Album importé',
    artist,
    artist_source: artist ? 'library_album' : 'unknown',
    play_duration_ms: 180_000,
    media_duration_ms: 180_000,
    start_position_ms: 0,
    end_position_ms: 180_000,
  },
})

test('une écoute de l\'historique sans artiste compte, se voit, et n\'invente rien', async ({ page, request }) => {
  const cookie = await apiSession(request)
  await send(
    request,
    [row(0, 'Titre importé', 'Groupe Importé'), row(5, 'Titre sans artiste', null), row(10, 'Titre sans artiste', null)],
    APPLE_TOKEN,
  )
  await waitForListens(request, cookie, date, 3)

  await login(page)
  await previousDay(page, DAYS_AGO)
  const music = page.getByTestId('music')
  await expect(page.getByTestId('listens')).toHaveText('3')
  await expect(page.getByTestId('without-artist')).toHaveText('dont 2 sans artiste connu.')
  await expect(page.getByTestId('minutes')).toHaveText('9 min')

  const rows = music.locator('.row')
  await expect(rows).toHaveCount(2)
  await expect(rows.first()).toContainText('Titre sans artiste')
  await expect(rows.first()).toContainText('Artiste inconnu')
  await checkLayout(page)
  await page.screenshot({ path: `${SHOTS}/historique-sans-artiste.png`, fullPage: true })

  await music.getByRole('button', { name: 'Artistes' }).click()
  await expect(rows).toHaveCount(1)
  await expect(rows.first()).toContainText('Groupe Importé')
  await expect(music.locator('.find')).toHaveCount(3)
})
