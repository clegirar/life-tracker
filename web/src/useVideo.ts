// Les vidéos YouTube d'un jour : lues à l'ouverture, au retour sur l'app et toutes les cinq minutes.
// L'historique n'arrive qu'une fois par jour : inutile de relire plus souvent.
// `enabled` : seulement quand la carte est affichée.

import { fetchDayVideo } from './api'
import { useDayPart } from './queries'

const REFRESH_MS = 5 * 60_000

export function useVideo(date: string, enabled: boolean) {
  const { data, loadFailed, reload } = useDayPart('video', fetchDayVideo, date, enabled, REFRESH_MS)
  return { video: data, loadFailed, reload }
}
