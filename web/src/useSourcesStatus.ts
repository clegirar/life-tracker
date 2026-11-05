// L'état des trackers : lu à la connexion (pour le point du menu), à chaque ouverture de la page, et,
// page ouverte, chaque minute et au retour au premier plan. Rien de plus : les connecteurs passent
// toutes les quelques minutes au mieux.

import { useCallback, useEffect, useRef, useState } from 'react'
import { fetchSourcesStatus, Unauthorized, type SourcesStatus } from './api'

const REFRESH_MS = 60_000

export function useSourcesStatus(onSessionLost: () => void, enabled: boolean, open: boolean) {
  const [status, setStatus] = useState<SourcesStatus | null>(null)
  const [loadFailed, setLoadFailed] = useState(false)
  const lastRequest = useRef(0)

  const refresh = useCallback(async () => {
    const request = ++lastRequest.current
    try {
      const value = await fetchSourcesStatus()
      if (request !== lastRequest.current) return
      setStatus(value)
      setLoadFailed(false)
    } catch (error) {
      if (error instanceof Unauthorized) {
        onSessionLost()
        return
      }
      if (request === lastRequest.current) setLoadFailed(true)
    }
  }, [onSessionLost])

  // À la connexion, et à chaque ouverture de la page.
  useEffect(() => {
    if (!enabled) return
    // Faux positif : `refresh` ne touche à l'état qu'après son `await`.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    void refresh()
  }, [enabled, open, refresh])

  useEffect(() => {
    if (!enabled || !open) return
    const whenVisible = () => {
      if (document.visibilityState === 'visible') void refresh()
    }
    const timer = setInterval(whenVisible, REFRESH_MS)
    document.addEventListener('visibilitychange', whenVisible)
    return () => {
      clearInterval(timer)
      document.removeEventListener('visibilitychange', whenVisible)
    }
  }, [enabled, open, refresh])

  const reset = useCallback(() => setStatus(null), [])

  return { status, loadFailed, reload: refresh, reset }
}
