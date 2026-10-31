// L'état de la page : ce que dit le serveur, les écritures en cours, et leur réconciliation.

import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  fetchDay,
  Refused,
  sendRecord,
  Transient,
  Unauthorized,
  type Cup,
  type OutgoingRecord,
} from './api'
import { failures, settle, unsettled, visibleCups, type Op } from './reconcile'
import { parisDate, parisTime, toRfc3339 } from './time'

/** Le worker lit le journal chaque seconde. */
const POLL_DELAYS_MS = [400, 700, 1000, 1500, 2000, 3000, 5000]
const SLOW_AFTER_MS = 15_000
const SEND_RETRY_DELAYS_MS = [500, 1500]

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms))

/** `crypto.randomUUID` n'existe qu'en contexte sécurisé, or le téléphone en dev passe par http. */
function newKey(): string {
  if (crypto.randomUUID) return crypto.randomUUID()
  const bytes = crypto.getRandomValues(new Uint8Array(16))
  bytes[6] = (bytes[6] & 0x0f) | 0x40
  bytes[8] = (bytes[8] & 0x3f) | 0x80
  const hex = Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('')
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`
}

export function useCoffee(date: string, onSessionLost: () => void, enabled: boolean) {
  const [server, setServer] = useState<Record<string, Cup[]>>({})
  const [ops, setOps] = useState<Op[]>([])
  const [loadFailed, setLoadFailed] = useState(false)
  const [slow, setSlow] = useState(false)
  // Une réponse plus ancienne que la dernière demandée ne remplace jamais celle-ci.
  const lastRequest = useRef(0)
  const lastApplied = useRef(0)
  const localKeys = useRef(new Set<string>())

  const refresh = useCallback(
    async (day: string) => {
      const request = ++lastRequest.current
      try {
        const { cups } = await fetchDay(day)
        if (request < lastApplied.current) return
        lastApplied.current = request
        // Dans la même mise à jour : le café passe des écritures au serveur sans clignoter.
        setServer((current) => ({ ...current, [day]: cups }))
        setOps((current) => settle(current, day, cups))
        setLoadFailed(false)
      } catch (error) {
        if (error instanceof Unauthorized) onSessionLost()
        else if (request === lastRequest.current) setLoadFailed(true)
      }
    },
    [onSessionLost],
  )

  useEffect(() => {
    if (!enabled) return
    // Faux positif : `refresh` ne touche à l'état qu'après son `await`.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    void refresh(date)
    const onVisible = () => {
      if (document.visibilityState === 'visible') void refresh(date)
    }
    document.addEventListener('visibilitychange', onVisible)
    return () => {
      document.removeEventListener('visibilitychange', onVisible)
    }
  }, [date, enabled, refresh])

  const cupsOnServer = server[date]
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

  const patch = useCallback((key: string, change: Partial<Op>) => {
    setOps((current) =>
      current.map((op) => (op.record.key === key ? { ...op, ...change } : op)),
    )
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

  const reset = useCallback(() => {
    setOps([])
    setServer({})
  }, [])

  return {
    /** `null` tant qu'on ne sait pas : jamais un zéro qui n'a pas été lu. */
    cups: cupsOnServer ? visibleCups(cupsOnServer, ops, date) : null,
    loadFailed,
    slow,
    failed: failures(ops),
    isLocal: (key: string) => localKeys.current.has(key),
    addCup,
    cancelCup,
    retry,
    dismiss,
    reload: () => refresh(date),
    reset,
  }
}
