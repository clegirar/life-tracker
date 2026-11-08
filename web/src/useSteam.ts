// Le Steam d'un jour : lu à l'ouverture, au retour sur l'app et toutes les 5 minutes. Le collecteur du
// NAS relève les compteurs de Steam tous les quarts d'heure : relire plus souvent n'apprendrait rien.
// `enabled` : seulement quand la carte est affichée.

import { fetchDayGames } from './api'
import { useDayPart } from './queries'

const REFRESH_MS = 5 * 60_000

export function useSteam(date: string, enabled: boolean) {
  const { data, loadFailed, reload } = useDayPart('games', fetchDayGames, date, enabled, REFRESH_MS)
  return { games: data, loadFailed, reload }
}
