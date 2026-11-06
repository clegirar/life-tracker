// Le Twitch d'un jour : lu à l'ouverture, au retour sur l'app et chaque minute. L'extension envoie ses
// segments au fil de la lecture, une minute au plus après : la carte d'aujourd'hui suit la soirée.

import { useCallback, useEffect, useRef, useState } from 'react'
import { fetchDayStream, Unauthorized, type DayStream } from './api'

const REFRESH_MS = 60_000

export function useTwitch(date: string, onSessionLost: () => void, enabled: boolean) {
  const [days, setDays] = useState<Record<string, DayStream>>({})
  const [loadFailed, setLoadFailed] = useState(false)
  const lastRequest = useRef(0)

  const refresh = useCallback(
    async (day: string) => {
      const request = ++lastRequest.current
      try {
        const stream = await fetchDayStream(day)
        if (request !== lastRequest.current) return
        setDays((current) => ({ ...current, [day]: stream }))
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
    /** `null` tant qu'on ne sait pas : jamais un zéro qui n'a pas été lu. */
    stream: days[date] ?? null,
    loadFailed,
    reload: () => refresh(date),
    reset,
  }
}
