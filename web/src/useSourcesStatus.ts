// L'état des trackers : lu à la connexion (pour le point du menu), à chaque ouverture de la page d'état
// ou des sources, et, page ouverte, chaque minute et au retour au premier plan. Rien de plus : les
// connecteurs passent toutes les quelques minutes au mieux.

import { useQuery } from '@tanstack/react-query'
import { loadFailed, statusQuery } from './queries'

const REFRESH_MS = 60_000

/** `open` : la page qui le montre, qui le relit à chaque ouverture. */
export function useSourcesStatus(enabled: boolean, open: boolean) {
  const query = useQuery({
    ...statusQuery,
    enabled,
    refetchInterval: open ? REFRESH_MS : false,
    refetchOnMount: open ? 'always' : true,
  })
  return { status: query.data ?? null, loadFailed: loadFailed(query), reload: query.refetch }
}
