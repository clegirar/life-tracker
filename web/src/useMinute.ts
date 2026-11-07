// L'heure, à la minute : de quoi faire avancer « Ici depuis 17 h 14 · 1 h 12 » sans rien relire. Un
// réveil à chaque changement de minute de l'horloge, et au retour sur l'app (un iPhone endormit ses
// minuteurs). Le chiffre change d'un coup, sans animation : c'est une donnée qu'on lit.

import { useSyncExternalStore } from 'react'

function subscribe(onChange: () => void): () => void {
  let timer: ReturnType<typeof setTimeout>
  const wake = () => {
    onChange()
    timer = setTimeout(wake, 60_000 - (Date.now() % 60_000))
  }
  timer = setTimeout(wake, 60_000 - (Date.now() % 60_000))
  const whenVisible = () => {
    if (document.visibilityState === 'visible') onChange()
  }
  document.addEventListener('visibilitychange', whenVisible)
  return () => {
    clearTimeout(timer)
    document.removeEventListener('visibilitychange', whenVisible)
  }
}

// Le début de la minute en cours : la même valeur d'un rendu à l'autre dans la minute.
const minute = () => Math.floor(Date.now() / 60_000) * 60_000

/** Le début de la minute en cours, en millisecondes, à jour à chaque minute d'horloge. */
export const useMinute = () => useSyncExternalStore(subscribe, minute)
