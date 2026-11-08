// Le Twitch d'un jour : lu à l'ouverture, au retour sur l'app et chaque minute. L'extension envoie ses
// segments au fil de la lecture, une minute au plus après : la carte d'aujourd'hui suit la soirée.
// `enabled` : seulement quand la carte est affichée.

import { fetchDayStream } from './api'
import { useDayPart } from './queries'

const REFRESH_MS = 60_000

export function useTwitch(date: string, enabled: boolean) {
  const { data, loadFailed, reload } = useDayPart('stream', fetchDayStream, date, enabled, REFRESH_MS)
  return { stream: data, loadFailed, reload }
}
