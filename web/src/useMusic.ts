// Les écoutes d'un jour, la santé des sources et les marquages « déjà connu » en cours.

import { useCallback, useEffect, useRef, useState } from 'react'
import {
  fetchDayMusic,
  fetchSourcesHealth,
  Refused,
  sendRecords,
  Transient,
  Unauthorized,
  type DayMusic,
  type FirstListen,
  type OutgoingRecord,
  type SourcesHealth,
} from './api'
import { cancelOp, itemId, markOp, settleOps, visibleFirstListens, type MarkOp } from './discoveries'

/** Les écoutes arrivent toutes les quelques minutes : inutile de relire plus souvent. */
const REFRESH_MS = 60_000
/** Le worker lit le journal chaque seconde. */
const POLL_DELAYS_MS = [400, 700, 1000, 1500, 2000, 3000, 5000]
const SLOW_AFTER_MS = 15_000
const SEND_RETRY_DELAYS_MS = [500, 1500]

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms))

export function useMusic(date: string, onSessionLost: () => void, enabled: boolean) {
  const [days, setDays] = useState<Record<string, DayMusic>>({})
  const [health, setHealth] = useState<SourcesHealth | null>(null)
  const [loadFailed, setLoadFailed] = useState(false)
  const [ops, setOps] = useState<MarkOp[]>([])
  const [slow, setSlow] = useState(false)
  const lastRequest = useRef(0)

  const refresh = useCallback(
    async (day: string) => {
      const request = ++lastRequest.current
      const [music, sources] = await Promise.allSettled([fetchDayMusic(day), fetchSourcesHealth()])
      if ([music, sources].some((r) => r.status === 'rejected' && r.reason instanceof Unauthorized)) {
        onSessionLost()
        return
      }
      // Une réponse plus ancienne que la dernière demandée ne compte plus.
      if (request !== lastRequest.current) return
      if (music.status === 'fulfilled') {
        // Dans la même mise à jour : le marquage passe des écritures au serveur sans clignoter.
        setDays((current) => ({ ...current, [day]: music.value }))
        setOps((current) => settleOps(current, day, music.value.first_listens))
      }
      if (sources.status === 'fulfilled') setHealth(sources.value)
      setLoadFailed(music.status === 'rejected')
    },
    [onSessionLost],
  )

  useEffect(() => {
    if (!enabled) return
    // Faux positif : `refresh` ne touche à l'état qu'après son `await`.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    void refresh(date)
    const whenVisible = () => {
      if (document.visibilityState === 'visible') void refresh(date)
    }
    const timer = setInterval(whenVisible, REFRESH_MS)
    document.addEventListener('visibilitychange', whenVisible)
    return () => {
      clearInterval(timer)
      document.removeEventListener('visibilitychange', whenVisible)
    }
  }, [date, enabled, refresh])

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
      await refresh(date)
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
  }, [date, enabled, refresh, waiting])

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

  const reset = useCallback(() => {
    setDays({})
    setHealth(null)
    setOps([])
  }, [])

  const day = days[date] ?? null

  return {
    /** `null` tant qu'on ne sait pas : jamais un zéro qui n'a pas été lu. */
    music: day,
    health,
    firstListens: day ? visibleFirstListens(day.first_listens, ops, date) : [],
    failedMarks: ops.filter((op) => op.status === 'failed'),
    marksSlow: slow,
    toggleKnown,
    retryMark,
    dismissMark,
    loadFailed,
    reload: () => refresh(date),
    reset,
  }
}
