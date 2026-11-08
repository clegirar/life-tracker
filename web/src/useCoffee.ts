// L'état de la page : ce que dit le serveur, les écritures en cours, et leur réconciliation.
// L'écran est optimiste : un café ajouté ou annulé se voit tout de suite (`visibleCups`), avant même la
// réponse du serveur ; un échec rend l'état du serveur et le dit à part. La lecture du jour vient du
// cache de TanStack Query (clé `['day', date]`, partagée avec la sonde de connexion et la lecture
// d'avance) ; les écritures gardent leur propre file, qui survit à l'écran de connexion.

import { useQueryClient, useQuery } from '@tanstack/react-query'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { newKey, Refused, sendRecord, Transient, Unauthorized, type Cup, type Day, type OutgoingRecord } from './api'
import { dayQuery, loadFailed } from './queries'
import { failures, settle, unsettled, visibleCups, type Op } from './reconcile'
import { parisDate, parisTime, toRfc3339 } from './time'

/** Le worker lit le journal chaque seconde. */
const POLL_DELAYS_MS = [400, 700, 1000, 1500, 2000, 3000, 5000]
const SLOW_AFTER_MS = 15_000
const SEND_RETRY_DELAYS_MS = [500, 1500]

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms))

export function useCoffee(date: string, onSessionLost: () => void, enabled: boolean) {
  const client = useQueryClient()
  const query = useQuery({ ...dayQuery(date), enabled })
  const [ops, setOps] = useState<Op[]>([])
  const [slow, setSlow] = useState(false)
  const localKeys = useRef(new Set<string>())

  // Chaque lecture d'un jour, d'où qu'elle vienne, retire les ajouts que le serveur montre désormais.
  // `visibleCups` ne compte jamais deux fois un café : entre la réponse et ce retrait, rien ne clignote.
  useEffect(
    () =>
      client.getQueryCache().subscribe((event) => {
        if (event.type !== 'updated' || event.action.type !== 'success') return
        const [scope, day, part] = event.query.queryKey
        const data = event.query.state.data as Day | undefined
        if (scope !== 'day' || typeof day !== 'string' || part !== undefined || !data) return
        setOps((current) => {
          const next = settle(current, day, data.cups)
          return next.length === current.length ? current : next
        })
      }),
    [client],
  )

  const cupsOnServer = query.data?.cups
  const waiting = useMemo(
    () =>
      unsettled(cupsOnServer, ops, date)
        .map((op) => op.record.key)
        .join(),
    [cupsOnServer, ops, date],
  )

  // Tant qu'une écriture reçue n'est pas visible, on relit le jour, de moins en moins souvent.
  useEffect(() => {
    if (!enabled || !waiting) return
    let step = 0
    let timer: ReturnType<typeof setTimeout>
    const tick = async () => {
      await client.fetchQuery({ ...dayQuery(date), staleTime: 0 }).catch(() => undefined)
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

  const patch = useCallback((key: string, change: Partial<Op>) => {
    setOps((current) => current.map((op) => (op.record.key === key ? { ...op, ...change } : op)))
  }, [])

  /** Même clé à chaque essai : le serveur répond `duplicate` si le premier était passé. */
  const deliver = useCallback(
    async (record: OutgoingRecord) => {
      for (let attempt = 0; ; attempt++) {
        try {
          await sendRecord(record)
          patch(record.key, { status: 'sent' })
          return
        } catch (error) {
          if (error instanceof Refused) {
            patch(record.key, { status: 'failed', final: true })
            return
          }
          if (error instanceof Unauthorized) {
            patch(record.key, { status: 'failed' })
            onSessionLost()
            return
          }
          if (!(error instanceof Transient) || attempt >= SEND_RETRY_DELAYS_MS.length) {
            patch(record.key, { status: 'failed' })
            return
          }
          await sleep(SEND_RETRY_DELAYS_MS[attempt])
        }
      }
    },
    [onSessionLost, patch],
  )

  const submit = useCallback(
    (op: Op) => {
      setOps((current) => [...current, op])
      void deliver(op.record)
    },
    [deliver],
  )

  const addCup = useCallback(
    (instant: number) => {
      const key = newKey()
      localKeys.current.add(key)
      submit({
        date: parisDate(instant),
        time: parisTime(instant),
        status: 'sending',
        record: {
          key,
          kind: 'coffee.cup',
          schema_version: 1,
          occurred_at: toRfc3339(instant),
          payload: {},
        },
      })
    },
    [submit],
  )

  const cancelCup = useCallback(
    (cup: Cup) => {
      submit({
        date,
        time: cup.time,
        status: 'sending',
        record: {
          key: newKey(),
          kind: 'record.cancel',
          schema_version: 1,
          occurred_at: toRfc3339(Date.now()),
          payload: { source: cup.source, key: cup.key },
        },
      })
    },
    [date, submit],
  )

  const retry = useCallback(
    (op: Op) => {
      patch(op.record.key, { status: 'sending' })
      void deliver(op.record)
    },
    [deliver, patch],
  )

  const dismiss = useCallback((op: Op) => {
    setOps((current) => current.filter((other) => other.record.key !== op.record.key))
  }, [])

  /** À la déconnexion ; le cache des lectures, lui, est vidé par l'app. */
  const reset = useCallback(() => setOps([]), [])

  return {
    /** `null` tant qu'on ne sait pas : jamais un zéro qui n'a pas été lu. */
    cups: cupsOnServer ? visibleCups(cupsOnServer, ops, date) : null,
    loadFailed: loadFailed(query),
    slow,
    failed: failures(ops),
    isLocal: (key: string) => localKeys.current.has(key),
    addCup,
    cancelCup,
    retry,
    dismiss,
    reload: query.refetch,
    reset,
  }
}
