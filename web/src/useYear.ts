// La vue d'une année, sur le modèle du mois : une année déjà lue s'affiche tout de suite, et elle est
// relue à chaque ouverture de la vue, dès qu'elle a vieilli, chaque minute vue ouverte et au retour au
// premier plan. Passer d'une année à l'autre ne relit qu'une année vieillie.

import { useQuery, useQueryClient } from '@tanstack/react-query'
import { loadFailed, yearQuery } from './queries'

const REFRESH_MS = 60_000

export function useYear(year: string) {
  const client = useQueryClient()
  const query = useQuery({ ...yearQuery(client, year), refetchInterval: REFRESH_MS, refetchOnMount: 'always' })
  return {
    /** `null` tant que cette année n'a jamais été lue : jamais les valeurs d'une autre. */
    year: query.data ?? null,
    loadFailed: loadFailed(query),
    reload: query.refetch,
  }
}
