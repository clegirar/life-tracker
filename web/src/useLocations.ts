// Les déplacements d'un jour : lus à l'ouverture, au retour sur l'app et toutes les 5 minutes. Overland
// envoie ses points par lots, au rythme de l'iPhone : relire plus souvent n'apprendrait presque rien.
// Une relecture identique garde l'objet précédent : la carte ne se redessine pas pour rien.

import { useCallback, useEffect, useRef, useState } from 'react'
import { fetchDayLocations, Unauthorized, type DayLocations } from './api'

const REFRESH_MS = 5 * 60_000

export function useLocations(date: string, onSessionLost: () => void, enabled: boolean) {
  const [days, setDays] = useState<Record<string, DayLocations>>({})
  const [loadFailed, setLoadFailed] = useState(false)
  const lastRequest = useRef(0)

  const refresh = useCallback(
    async (day: string) => {
      const request = ++lastRequest.current
      try {
        const locations = await fetchDayLocations(day)
        if (request !== lastRequest.current) return
        setDays((current) =>
          current[day] && JSON.stringify(current[day]) === JSON.stringify(locations) ? current : { ...current, [day]: locations },
        )
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
    /** `null` tant qu'on ne sait pas : jamais un « aucun déplacement » qui n'a pas été lu. */
    locations: days[date] ?? null,
    loadFailed,
    reload: () => refresh(date),
    reset,
  }
}
