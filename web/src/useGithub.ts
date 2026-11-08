// Le GitHub d'un jour : lu à l'ouverture, au retour sur l'app et toutes les 5 minutes. Le collecteur du
// NAS interroge GitHub tous les quarts d'heure : relire plus souvent n'apprendrait rien.
// `enabled` : seulement quand la carte est affichée.

import { fetchDayGithub } from './api'
import { useDayPart } from './queries'

const REFRESH_MS = 5 * 60_000

export function useGithub(date: string, enabled: boolean) {
  const { data, loadFailed, reload } = useDayPart('github', fetchDayGithub, date, enabled, REFRESH_MS)
  return { github: data, loadFailed, reload }
}
