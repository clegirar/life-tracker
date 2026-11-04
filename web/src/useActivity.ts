// Les données Garmin d'un jour : lues à l'ouverture, au retour sur l'app et toutes les cinq minutes.
// Le connecteur passe tous les quarts d'heure : inutile de relire plus souvent.

import { useCallback, useEffect, useRef, useState } from 'react'
import { fetchDayActivity, Unauthorized, type DayActivity } from './api'

const REFRESH_MS = 5 * 60_000

export function useActivity(date: string, onSessionLost: () => void, enabled: boolean) {
  const [days, setDays] = useState<Record<string, DayActivity>>({})
  const [loadFailed, setLoadFailed] = useState(false)
  const lastRequest = useRef(0)

  const refresh = useCallback(
    async (day: string) => {
      const request = ++lastRequest.current
      try {
        const activity = await fetchDayActivity(day)
        if (request !== lastRequest.current) return
        setDays((current) => ({ ...current, [day]: activity }))
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

  const reset = useCallback(() => {
    setDays({})
    setLoadFailed(false)
  }, [])

  return {
    activity: days[date] ?? null,
    loadFailed,
    reload: () => refresh(date),
    reset,
  }
}
