// Les lectures du backend, par TanStack Query : un cache par clé (le jour, le mois, l'année…), relu
// quand il a vieilli, au retour sur l'app et à l'intervalle de chaque vue. Une clé ne porte jamais que
// ses propres données : une période qui n'est pas encore lue s'affiche vide et en attente, jamais avec
// les valeurs d'une autre (une journée sans données ne se confond pas avec une journée sans activité).

import { QueryCache, QueryClient, queryOptions, useQuery, useQueryClient, type UseQueryResult } from '@tanstack/react-query'
import { useCallback } from 'react'
import {
  fetchDay,
  fetchMonth,
  fetchSourcesHealth,
  fetchSourcesStatus,
  fetchYear,
  Transient,
  Unauthorized,
  type MonthMetric,
} from './api'

/** Une donnée lue depuis moins longtemps se montre telle quelle, sans relecture. */
const STALE_MS = 30_000
/** Une période quittée reste en mémoire dix minutes : y revenir l'affiche tout de suite. */
const GC_MS = 10 * 60_000
/** Une panne passagère (réseau, 5xx) se retente deux fois ; le reste échoue tout de suite. */
const RETRY_DELAYS_MS = [500, 1500]

export const retryTransient = (failureCount: number, error: unknown) =>
  error instanceof Transient && failureCount < RETRY_DELAYS_MS.length

export const keys = {
  /** Le jour : ses cafés. Ses autres lectures (`dayPart`) vivent dessous. */
  day: (date: string) => ['day', date] as const,
  dayPart: (part: string, date: string) => ['day', date, part] as const,
  month: (month: string) => ['month', month] as const,
  year: (year: string) => ['year', year] as const,
  health: ['sources', 'health'] as const,
  status: ['sources', 'status'] as const,
  /** La dernière liste de métriques connue, de n'importe quel mois ou année lu. */
  metrics: ['metrics'] as const,
}

// La session perdue se dit à l'app, qui montre l'écran de connexion. Une seule app, un seul abonné.
const sessionLostListeners = new Set<() => void>()

export function onSessionLost(listener: () => void): () => void {
  sessionLostListeners.add(listener)
  return () => {
    sessionLostListeners.delete(listener)
  }
}

export function createQueryClient(): QueryClient {
  return new QueryClient({
    queryCache: new QueryCache({
      onError: (error) => {
        if (error instanceof Unauthorized) for (const listener of sessionLostListeners) listener()
      },
    }),
    defaultOptions: {
      queries: {
        staleTime: STALE_MS,
        gcTime: GC_MS,
        retry: retryTransient,
        retryDelay: (attempt) => RETRY_DELAYS_MS[Math.min(attempt, RETRY_DELAYS_MS.length - 1)],
        // `visibilitychange` : le retour sur l'app relit ce qui a vieilli.
        refetchOnWindowFocus: true,
        // Les intervalles ne tournent qu'app visible, comme avant.
        refetchIntervalInBackground: false,
      },
    },
  })
}

/** La dernière lecture a échoué. Une session perdue n'en est pas une : l'écran de connexion la dit. */
export const loadFailed = (query: Pick<UseQueryResult, 'isError' | 'error'>) =>
  query.isError && !(query.error instanceof Unauthorized)

export const dayQuery = (date: string) => queryOptions({ queryKey: keys.day(date), queryFn: () => fetchDay(date) })

// La liste des métriques (le registre, sans aucune valeur) est gardée d'une visite à l'autre : les
// filtres, la légende et le type de case du mois ou de l'année ne dépendent plus de la première réponse.
const METRICS_STORAGE_KEY = 'calendar.metric-list'

function readStoredMetrics(): MonthMetric[] {
  try {
    const value: unknown = JSON.parse(window.localStorage.getItem(METRICS_STORAGE_KEY) ?? '[]')
    return Array.isArray(value) && value.every((m) => typeof m?.metric === 'string' && typeof m?.label === 'string') ? value : []
  } catch {
    return []
  }
}

function rememberMetrics(client: QueryClient, metrics: MonthMetric[]) {
  const known = client.getQueryData<MonthMetric[]>(keys.metrics)
  if (known && JSON.stringify(known) === JSON.stringify(metrics)) return
  client.setQueryData(keys.metrics, metrics)
  try {
    window.localStorage.setItem(METRICS_STORAGE_KEY, JSON.stringify(metrics))
  } catch {
    // Navigation privée : la liste vaut pour cette visite.
  }
}

export const monthQuery = (client: QueryClient, month: string) =>
  queryOptions({
    queryKey: keys.month(month),
    queryFn: async () => {
      const value = await fetchMonth(month)
      rememberMetrics(client, value.metrics)
      return value
    },
  })

export const yearQuery = (client: QueryClient, year: string) =>
  queryOptions({
    queryKey: keys.year(year),
    queryFn: async () => {
      const value = await fetchYear(year)
      rememberMetrics(client, value.metrics)
      return value
    },
  })

/** Les métriques du registre, telles que la dernière lecture d'un mois ou d'une année les a dites. Vide : jamais lues. */
export function useKnownMetrics(): MonthMetric[] {
  return useQuery({
    queryKey: keys.metrics,
    queryFn: readStoredMetrics,
    initialData: readStoredMetrics,
    staleTime: Infinity,
    gcTime: Infinity,
  }).data
}

/** Une lecture d'avance, ignorée si la donnée est encore fraîche. */
export function usePrefetch() {
  const client = useQueryClient()
  return {
    month: useCallback((month: string) => void client.prefetchQuery(monthQuery(client, month)), [client]),
    year: useCallback((year: string) => void client.prefetchQuery(yearQuery(client, year)), [client]),
    day: useCallback((date: string) => void client.prefetchQuery(dayQuery(date)), [client]),
  }
}

// La santé des sources lit tout l'historique : seules les pages qui la montrent (le mois, le jour) la
// demandent, toutes les cinq minutes au plus.
const HEALTH_REFRESH_MS = 5 * 60_000

export function useHealth() {
  return useQuery({
    queryKey: keys.health,
    queryFn: fetchSourcesHealth,
    refetchInterval: HEALTH_REFRESH_MS,
    staleTime: 60_000,
  }).data ?? null
}

/** Une lecture d'un jour, par carte : `enabled` seulement si la carte est affichée. */
export function useDayPart<T>(part: string, fetcher: (date: string) => Promise<T>, date: string, enabled: boolean, refreshMs: number) {
  const query = useQuery({
    queryKey: keys.dayPart(part, date),
    queryFn: () => fetcher(date),
    enabled,
    refetchInterval: refreshMs,
  })
  return {
    /** `null` tant que ce jour n'a jamais été lu : jamais un zéro qui n'a pas été lu. */
    data: query.data ?? null,
    loadFailed: loadFailed(query),
    reload: query.refetch,
  }
}

export const statusQuery = queryOptions({ queryKey: keys.status, queryFn: fetchSourcesStatus })
