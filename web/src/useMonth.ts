// La vue d'un mois. Un mois déjà lu s'affiche tout de suite, et il est relu dès qu'il a vieilli, et à
// chaque ouverture de la vue : un jour passé peut changer d'apparence (donnée tardive, import,
// annulation, erreur). Passer d'un mois à l'autre dans la vue ne relit qu'un mois vieilli : un voisin lu
// d'avance s'affiche sans nouvelle lecture.

import { useQuery, useQueryClient } from '@tanstack/react-query'
import { loadFailed, monthQuery } from './queries'

const REFRESH_MS = 60_000

/** `view` : la vue du mois elle-même, relue à chaque ouverture ; la page du jour n'en lit qu'un jour. */
export function useMonth(month: string, view = false) {
  const client = useQueryClient()
  const query = useQuery({ ...monthQuery(client, month), refetchInterval: REFRESH_MS, refetchOnMount: view ? 'always' : true })
  return {
    /** `null` tant que ce mois n'a jamais été lu : jamais les valeurs d'un autre. */
    month: query.data ?? null,
    loadFailed: loadFailed(query),
    reload: query.refetch,
  }
}
