// Les écoutes d'un jour et les marquages « déjà connu » en cours. Le marquage est optimiste : il se voit
// tout de suite (`visibleFirstListens`), un échec rend l'état du serveur et le dit à part. La santé des
// sources, longue à calculer, n'est plus lue ici : seules les pages qui la montrent la demandent
// (`useHealth`).

import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useCallback, useEffect, useState } from 'react'
import { fetchDayMusic, Refused, sendRecords, Transient, Unauthorized, type DayMusic, type FirstListen, type OutgoingRecord } from './api'
import { cancelOp, itemId, markOp, settleOps, visibleFirstListens, type MarkOp } from './discoveries'
import { keys, loadFailed } from './queries'

/** Les écoutes arrivent toutes les quelques minutes : inutile de relire plus souvent. */
const REFRESH_MS = 60_000
/** Le worker lit le journal chaque seconde. */
const POLL_DELAYS_MS = [400, 700, 1000, 1500, 2000, 3000, 5000]
const SLOW_AFTER_MS = 15_000
const SEND_RETRY_DELAYS_MS = [500, 1500]

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms))

const musicQuery = (date: string) => ({ queryKey: keys.dayPart('music', date), queryFn: () => fetchDayMusic(date) })

export function useMusic(date: string, onSessionLost: () => void, enabled: boolean) {
  const client = useQueryClient()
  const query = useQuery({ ...musicQuery(date), enabled, refetchInterval: REFRESH_MS })
  const [ops, setOps] = useState<MarkOp[]>([])
  const [slow, setSlow] = useState(false)

  // Chaque lecture des écoutes d'un jour retire les marquages que le serveur montre désormais.
  useEffect(
    () =>
      client.getQueryCache().subscribe((event) => {
        if (event.type !== 'updated' || event.action.type !== 'success') return
        const [scope, day, part] = event.query.queryKey
        const data = event.query.state.data as DayMusic | undefined
        if (scope !== 'day' || typeof day !== 'string' || part !== 'music' || !data) return
        setOps((current) => {
          const next = settleOps(current, day, data.first_listens)
          return next.length === current.length ? current : next
        })
      }),
    [client],
  )

  const waiting = ops
    .filter((op) => op.date === date && op.status !== 'failed')
    .map((op) => op.id)
    .join()

  // Tant qu'un marquage n'est pas visible côté serveur, on relit le jour, de moins en moins souvent.
  useEffect(() => {
    if (!enabled || !waiting) return
    let step = 0
    let timer: ReturnType<typeof setTimeout>
    const tick = async () => {
      await client.fetchQuery({ ...musicQuery(date), staleTime: 0 }).catch(() => undefined)
      step = Math.min(step + 1, POLL_DELAYS_MS.length - 1)
      timer = setTimeout(tick, POLL_DELAYS_MS[step])
    }
    timer = setTimeout(tick, POLL_DELAYS_MS[0])
    const slowTimer = setTimeout(() => setSlow(true), SLOW_AFTER_MS)
    return () => {
      clearTimeout(timer)
      clearTimeout(slowTimer)
      setSlow(false)
    }
  }, [client, date, enabled, waiting])

  const patch = useCallback((id: string, change: Partial<MarkOp>) => {
    setOps((current) => current.map((op) => (op.id === id ? { ...op, ...change } : op)))
  }, [])

  /** Mêmes clés à chaque essai : le serveur répond `duplicate` si le premier était passé. */
  const deliver = useCallback(
    async (id: string, records: OutgoingRecord[]) => {
      for (let attempt = 0; ; attempt++) {
        try {
          await sendRecords(records)
          patch(id, { status: 'sent' })
          return
        } catch (error) {
          if (error instanceof Unauthorized) {
            patch(id, { status: 'failed' })
            onSessionLost()
            return
          }
          if (error instanceof Refused) {
            patch(id, { status: 'failed', final: true })
            return
          }
          if (!(error instanceof Transient) || attempt >= SEND_RETRY_DELAYS_MS.length) {
            patch(id, { status: 'failed' })
            return
          }
          await sleep(SEND_RETRY_DELAYS_MS[attempt])
        }
      }
    },
    [onSessionLost, patch],
  )

  /** Marque, ou annule tous les marquages de l'élément. Un geste à la fois par élément. */
  const toggleKnown = (row: FirstListen) => {
    const id = itemId(row)
    const running = ops.find((op) => op.id === id)
    if (running && running.status !== 'failed') return
    const op = row.known_marks.length > 0 ? cancelOp(date, row) : markOp(date, row)
    setOps((current) => [...current.filter((other) => other.id !== id), op])
    void deliver(id, op.records)
  }

  const retryMark = (op: MarkOp) => {
    patch(op.id, { status: 'sending', final: false })
    void deliver(op.id, op.records)
  }

  const dismissMark = (op: MarkOp) => {
    setOps((current) => current.filter((other) => other.id !== op.id))
  }

  /** À la déconnexion ; le cache des lectures, lui, est vidé par l'app. */
  const reset = useCallback(() => setOps([]), [])

  const day = query.data ?? null

  return {
    /** `null` tant qu'on ne sait pas : jamais un zéro qui n'a pas été lu. */
    music: day,
    firstListens: day ? visibleFirstListens(day.first_listens, ops, date) : [],
    failedMarks: ops.filter((op) => op.status === 'failed'),
    marksSlow: slow,
    toggleKnown,
    retryMark,
    dismissMark,
    loadFailed: loadFailed(query),
    reload: query.refetch,
    reset,
  }
}
