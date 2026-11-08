// L'arrivée des valeurs d'une période attendue (un mois, une année pas encore lus) : elles se fondent,
// la grille, elle, était déjà là. Une période déjà en cache s'affiche d'un coup, sans fondu : rien n'a
// été attendu.

import { useState } from 'react'

/** Vrai une fois arrivées les données de `key`, si on les a vues en attente. */
export function useReveal(key: string, ready: boolean): boolean {
  const [waited, setWaited] = useState<string | null>(null)
  // Ajusté pendant le rendu, comme le recommande React pour un état qui suit une prop.
  if (!ready && waited !== key) setWaited(key)
  if (ready && waited !== null && waited !== key) setWaited(null)
  return ready && waited === key
}
