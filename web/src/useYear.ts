// La vue d'une année, sur le modèle du mois : une année déjà lue s'affiche tout de suite, mais elle est
// relue à chaque ouverture, chaque minute et au retour au premier plan. Rien n'est gardé pour de bon.

import { useCallback, useEffect, useRef, useState } from 'react'
import { fetchYear, Unauthorized, type Year } from './api'

const REFRESH_MS = 60_000

/** `enabled` seulement quand la vue est ouverte : la rouvrir relit l'année. */
export function useYear(year: string, onSessionLost: () => void, enabled: boolean) {
  const [years, setYears] = useState<Record<string, Year>>({})
  const [loadFailed, setLoadFailed] = useState(false)
  const lastRequest = useRef(0)

  const refresh = useCallback(
    async (wanted: string) => {
      const request = ++lastRequest.current
      try {
        const value = await fetchYear(wanted)
        if (request !== lastRequest.current) return
        setYears((current) => ({ ...current, [wanted]: value }))
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
    void refresh(year)
    const whenVisible = () => {
      if (document.visibilityState === 'visible') void refresh(year)
    }
    const timer = setInterval(whenVisible, REFRESH_MS)
    document.addEventListener('visibilitychange', whenVisible)
    return () => {
      clearInterval(timer)
      document.removeEventListener('visibilitychange', whenVisible)
    }
  }, [year, enabled, refresh])

  const reset = useCallback(() => setYears({}), [])

  return {
    /** `null` tant que ce mois n'a jamais été lu. */
    year: years[year] ?? null,
    loadFailed,
    reload: () => refresh(year),
    reset,
  }
}
