// Les métriques affichées par le calendrier : un seul choix pour le mois et l'année, gardé d'une visite
// à l'autre. Une seule : ses chiffres dans le mois, son intensité dans l'année. Plusieurs : des points
// façon Calendrier d'Apple dans les deux vues, un par métrique active ce jour-là.

import { useCallback, useState } from 'react'
import type { MonthDay } from './api'

const STORAGE_KEY = 'calendar.metrics'

// La couleur de chaque métrique connue ; une métrique nouvelle prend la teinte neutre, sans code à changer.
const TONES: Record<string, string> = {
  coffee: 'heat-coffee',
  'music.listen': 'heat-music',
  steps: 'heat-steps',
  'calories.burned': 'heat-calories',
  sleep: 'heat-sleep',
  workout: 'heat-workout',
  'video.watch': 'heat-video',
}

/** La classe qui pose `--heat`, la couleur de la métrique : chiffres, points, pastilles, filtres et rampes. */
export const metricTone = (metric: string) => TONES[metric] ?? 'heat-neutral'

/** Ce qu'une journée dit en mode points : pas de données du tout, ou les métriques actives (valeur > 0). */
export interface DayActivity {
  /** Aucune des métriques choisies n'a de données ce jour-là : le numéro est grisé. */
  noData: boolean
  /** Les métriques actives, dans l'ordre du registre ; vide un jour connu sans activité. */
  active: string[]
}

export function dayActivity(day: MonthDay | undefined, metrics: string[]): DayActivity {
  const value = (metric: string) => day?.values[metric] ?? null
  return {
    noData: metrics.every((metric) => value(metric) === null),
    active: metrics.filter((metric) => (value(metric) ?? 0) > 0),
  }
}

/** Les points qu'on dessine, `max` au plus, et combien restent pour « +N ». */
export function visibleDots(active: string[], max: number): { dots: string[]; more: number } {
  return { dots: active.slice(0, max), more: Math.max(0, active.length - max) }
}

/** Le choix gardé, réduit aux métriques connues et dans leur ordre ; vide ou inconnu : toutes. */
export function effectiveSelection(stored: string[] | null, available: string[]): string[] {
  const kept = available.filter((metric) => stored?.includes(metric))
  return kept.length > 0 ? kept : available
}

/** Ajoute ou retire une métrique ; la dernière choisie reste : le calendrier montre toujours quelque chose. */
export function toggleMetric(current: string[], metric: string, available: string[]): string[] {
  if (current.includes(metric)) {
    return current.length === 1 ? current : current.filter((m) => m !== metric)
  }
  return available.filter((m) => m === metric || current.includes(m))
}

function readStored(): string[] | null {
  try {
    const value: unknown = JSON.parse(window.localStorage.getItem(STORAGE_KEY) ?? 'null')
    return Array.isArray(value) && value.every((m) => typeof m === 'string') ? value : null
  } catch {
    return null
  }
}

/** Le choix commun au mois et à l'année. `null` : jamais choisi, donc toutes les métriques. */
export function useMetricSelection() {
  const [stored, setStored] = useState<string[] | null>(readStored)

  const toggle = useCallback((metric: string, available: string[]) => {
    setStored((current) => {
      const next = toggleMetric(effectiveSelection(current, available), metric, available)
      try {
        window.localStorage.setItem(STORAGE_KEY, JSON.stringify(next))
      } catch {
        // Navigation privée : le choix vaut pour cette visite.
      }
      return next
    })
  }, [])

  return { stored, toggle }
}
