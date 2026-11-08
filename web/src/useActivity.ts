// Les données Garmin d'un jour : lues à l'ouverture, au retour sur l'app et toutes les cinq minutes.
// Le connecteur passe tous les quarts d'heure : inutile de relire plus souvent.
// `enabled` : seulement quand la carte est affichée.

import { fetchDayActivity } from './api'
import { useDayPart } from './queries'

const REFRESH_MS = 5 * 60_000

export function useActivity(date: string, enabled: boolean) {
  const { data, loadFailed, reload } = useDayPart('activity', fetchDayActivity, date, enabled, REFRESH_MS)
  return { activity: data, loadFailed, reload }
}
