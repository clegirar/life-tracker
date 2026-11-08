// Les déplacements d'un jour : lus à l'ouverture, au retour sur l'app et toutes les 5 minutes. Overland
// envoie ses points par lots, au rythme de l'iPhone : relire plus souvent n'apprendrait presque rien.
// Une relecture identique garde l'objet précédent (partage structurel de TanStack Query) : la carte ne
// se redessine pas pour rien.
// `enabled` : seulement quand la carte est affichée.

import { fetchDayLocations } from './api'
import { useDayPart } from './queries'

const REFRESH_MS = 5 * 60_000

export function useLocations(date: string, enabled: boolean) {
  const { data, loadFailed, reload } = useDayPart('locations', fetchDayLocations, date, enabled, REFRESH_MS)
  return { locations: data, loadFailed, reload }
}
