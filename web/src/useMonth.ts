// La vue d'un mois. Un mois déjà lu s'affiche tout de suite, mais il est toujours relu : un jour passé
// peut changer d'apparence (donnée tardive, import, annulation, erreur), rien n'est gardé pour de bon.

import { useCallback, useEffect, useRef, useState } from 'react'
import { fetchMonth, Unauthorized, type Month } from './api'

const REFRESH_MS = 60_000

/** `shown` : chaque fois que la vue s'ouvre de nouveau, le mois est relu. */
export function useMonth(month: string, onSessionLost: () => void, enabled: boolean, shown: unknown) {
  const [months, setMonths] = useState<Record<string, Month>>({})
  const [loadFailed, setLoadFailed] = useState(false)
  const lastRequest = useRef(0)

  const refresh = useCallback(
    async (wanted: string) => {
      const request = ++lastRequest.current
      try {
        const value = await fetchMonth(wanted)
        if (request !== lastRequest.current) return
        setMonths((current) => ({ ...current, [wanted]: value }))
        setLoadFailed(false)
      } catch (error) {
        if (error instanceof Unauthorized) {
          onSessionLost()
          return
        }
        if (request === lastRequest.current) setLoadFailed(true)
      }
    },
    [onSessionLost],
  )

  useEffect(() => {
    if (!enabled) return
    // Faux positif : `refresh` ne touche à l'état qu'après son `await`.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    void refresh(month)
    const whenVisible = () => {
      if (document.visibilityState === 'visible') void refresh(month)
    }
    const timer = setInterval(whenVisible, REFRESH_MS)
    document.addEventListener('visibilitychange', whenVisible)
    return () => {
      clearInterval(timer)
      document.removeEventListener('visibilitychange', whenVisible)
    }
  }, [month, enabled, refresh, shown])

  const reset = useCallback(() => setMonths({}), [])

  return {
    /** `null` tant que ce mois n'a jamais été lu. */
    month: months[month] ?? null,
    loadFailed,
    reload: () => refresh(month),
    reset,
  }
}
