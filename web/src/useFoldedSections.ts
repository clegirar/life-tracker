// Les sections pliées de la carte « Jeux » : un réglage du lecteur, gardé dans le navigateur et valable
// pour tous les jours. Sans stockage (navigation privée, données bloquées), tout s'ouvre et le pli vaut
// pour la visite.

import { useCallback, useState } from 'react'
import { parseFolded, toggleFolded, type GameSectionId } from './games'

const STORAGE_KEY = 'games.folded'

function read(): GameSectionId[] {
  try {
    return parseFolded(window.localStorage.getItem(STORAGE_KEY))
  } catch {
    return []
  }
}

export function useFoldedSections() {
  const [folded, setFolded] = useState<GameSectionId[]>(read)
  const toggle = useCallback((id: GameSectionId) => {
    setFolded((current) => {
      const next = toggleFolded(current, id)
      try {
        window.localStorage.setItem(STORAGE_KEY, JSON.stringify(next))
      } catch {
        // Stockage indisponible : le pli vaut pour cette visite.
      }
      return next
    })
  }, [])
  return { folded, toggle }
}
