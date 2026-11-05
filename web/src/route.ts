// Les adresses des pages : un petit routeur sur l'History API, sans dépendance.
// `/` suit aujourd'hui, y compris quand minuit passe avec la page ouverte ; une adresse invalide
// ou future renvoie vers `/`. nginx sert index.html pour toutes les adresses.

import { useCallback, useEffect, useState } from 'react'

export type Route =
  /** `date` à `null` : aujourd'hui, quel qu'il soit. */
  | { page: 'day'; date: string | null }
  | { page: 'month'; month: string }
  | { page: 'year'; year: string }
  | { page: 'status' }
  /** Le retour de Google après un accord ; sa requête (`state`, `code` ou `error`) reste dans l'adresse. */
  | { page: 'authorization' }

export const TODAY: Route = { page: 'day', date: null }
export const STATUS: Route = { page: 'status' }

/** Une vraie date du calendrier, au format strict. */
function isDate(value: string): boolean {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(value)) return false
  const [y, m, d] = value.split('-').map(Number)
  const date = new Date(Date.UTC(y, m - 1, d))
  return date.getUTCFullYear() === y && date.getUTCMonth() === m - 1 && date.getUTCDate() === d
}

/** La page d'une adresse, ou `null` si elle est invalide ou future. */
export function parseRoute(path: string, today: string): Route | null {
  const clean = path.length > 1 ? path.replace(/\/+$/, '') : path
  if (clean === '/') return TODAY
  if (clean === '/etat') return STATUS
  if (clean === '/autorisation') return { page: 'authorization' }
  let match = /^\/jour\/([^/]+)$/.exec(clean)
  if (match) {
    const date = match[1]
    if (!isDate(date) || date > today) return null
    return { page: 'day', date: date === today ? null : date }
  }
  match = /^\/mois\/(\d{4}-\d{2})$/.exec(clean)
  if (match) {
    const month = match[1]
    if (!isDate(`${month}-01`) || month > today.slice(0, 7)) return null
    return { page: 'month', month }
  }
  match = /^\/annee\/(\d{4})$/.exec(clean)
  if (match) {
    const year = match[1]
    if (year > today.slice(0, 4)) return null
    return { page: 'year', year }
  }
  return null
}

export function routePath(route: Route): string {
  switch (route.page) {
    case 'day':
      return route.date === null ? '/' : `/jour/${route.date}`
    case 'month':
      return `/mois/${route.month}`
    case 'year':
      return `/annee/${route.year}`
    case 'status':
      return '/etat'
    case 'authorization':
      return '/autorisation'
  }
}

const currentPath = () => window.location.pathname

/**
 * La page affichée et la navigation : `pushState` à chaque changement, `popstate` pour Retour.
 * Une adresse invalide est remplacée par `/`, sans entrée d'historique.
 */
export function useRoute(today: string) {
  const [path, setPath] = useState(currentPath)

  useEffect(() => {
    const onPop = () => setPath(currentPath())
    window.addEventListener('popstate', onPop)
    return () => window.removeEventListener('popstate', onPop)
  }, [])

  const parsed = parseRoute(path, today)
  const route = parsed ?? TODAY

  useEffect(() => {
    if (parsed === null) window.history.replaceState(null, '', '/')
  }, [parsed])

  /** `replace` : sans entrée d'historique, pour une déconnexion par exemple. */
  const navigate = useCallback((next: Route, replace = false) => {
    const target = routePath(next)
    if (target === currentPath() && !replace) return
    const fromKind = (parseRoute(currentPath(), '9999-12-31') ?? TODAY).page
    if (replace) window.history.replaceState(null, '', target)
    else window.history.pushState(null, '', target)
    setPath(target)
    // Une autre vue commence en haut ; un jour voisin garde la position, comme avant.
    if (fromKind !== next.page) window.scrollTo(0, 0)
  }, [])

  return { route, navigate }
}
