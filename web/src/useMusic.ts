// Les écoutes d'un jour et la santé des sources. En lecture seule : rien à réconcilier.

import { useCallback, useEffect, useRef, useState } from 'react'
import {
  fetchDayMusic,
  fetchSourcesHealth,
  Unauthorized,
  type DayMusic,
  type SourcesHealth,
} from './api'

/** Les écoutes arrivent toutes les quelques minutes : inutile de relire plus souvent. */
const REFRESH_MS = 60_000

export function useMusic(date: string, onSessionLost: () => void, enabled: boolean) {
  const [days, setDays] = useState<Record<string, DayMusic>>({})
  const [health, setHealth] = useState<SourcesHealth | null>(null)
  const [loadFailed, setLoadFailed] = useState(false)
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
        setDays((current) => ({ ...current, [day]: music.value }))
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

  const reset = useCallback(() => {
    setDays({})
    setHealth(null)
  }, [])

  return {
    /** `null` tant qu'on ne sait pas : jamais un zéro qui n'a pas été lu. */
    music: days[date] ?? null,
    health,
    loadFailed,
    reload: () => refresh(date),
    reset,
  }
}
