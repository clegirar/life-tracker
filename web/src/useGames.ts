// Les jeux d'un jour (Steam, Riot, VALORANT) : lus à l'ouverture, au retour sur l'app et toutes les
// 5 minutes. Les collecteurs du NAS passent tous les quarts d'heure : relire plus souvent n'apprendrait rien.
// `enabled` : seulement quand la carte est affichée.

import { fetchDayGames } from './api'
import { useDayPart } from './queries'

const REFRESH_MS = 5 * 60_000

export function useGames(date: string, enabled: boolean) {
  const { data, loadFailed, reload } = useDayPart('games', fetchDayGames, date, enabled, REFRESH_MS)
  return { games: data, loadFailed, reload }
}
