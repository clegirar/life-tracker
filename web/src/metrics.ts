// Les métriques affichées : un seul choix pour le mois, l'année et le jour, gardé d'une visite à
// l'autre. Une seule : ses chiffres dans le mois, son intensité dans l'année. Plusieurs : des points
// façon Calendrier d'Apple dans les deux vues, un par métrique active ce jour-là.
//
// Le choix porte toujours sur des métriques ; seules les pastilles des filtres en regroupent
// certaines (`METRIC_GROUPS`), cochées et décochées ensemble.

import { useCallback, useState } from 'react'
import type { MonthDay, MonthMetric } from './api'

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
  'stream.watch': 'heat-twitch',
  'game.play': 'heat-steam',
  'place.visit': 'heat-location',
  'code.commit': 'heat-github',
  'code.pull_request': 'heat-github',
}

/** La classe qui pose `--heat`, la couleur de la métrique : chiffres, points, pastilles, filtres et rampes. */
export const metricTone = (metric: string) => TONES[metric] ?? 'heat-neutral'

/** Des métriques qui n'ont qu'une pastille dans les filtres, à la place de la première présente. */
interface MetricGroup {
  label: string
  /** La couleur de la pastille : celle du titre de la carte du jour qui les montre. */
  tone: string
  metrics: string[]
}

// Garmin : une seule pastille « Activité » pour les quatre blocs de sa carte (à ma demande, 7 nov. 2026).
// GitHub : une seule pastille « GitHub » pour les commits et les pull requests, que sa carte montre
// ensemble (à ma demande, 8 nov. 2026).
const METRIC_GROUPS: MetricGroup[] = [
  { label: 'Activité', tone: 'heat-steps', metrics: ['steps', 'workout', 'sleep', 'calories.burned'] },
  { label: 'GitHub', tone: 'heat-github', metrics: ['code.commit', 'code.pull_request'] },
]

const groupOf = (metric: string) => METRIC_GROUPS.find((group) => group.metrics.includes(metric))

/** Une pastille des filtres : une métrique, ou un groupe, avec les métriques qu'elle coche. */
export interface MetricChip {
  /** La clé React et l'appui : la première métrique de la pastille. */
  id: string
  label: string
  tone: string
  metrics: string[]
}

/** Les pastilles, dans l'ordre du registre ; un groupe prend la place de sa première métrique présente. */
export function metricChips(metrics: MonthMetric[]): MetricChip[] {
  const chips: MetricChip[] = []
  for (const metric of metrics) {
    const group = groupOf(metric.metric)
    if (!group) {
      chips.push({ id: metric.metric, label: metric.label, tone: metricTone(metric.metric), metrics: [metric.metric] })
      continue
    }
    const chip = chips.find((c) => groupOf(c.id) === group)
    if (chip) chip.metrics.push(metric.metric)
    else chips.push({ id: metric.metric, label: group.label, tone: group.tone, metrics: [metric.metric] })
  }
  return chips
}

/** Les métriques qu'une pastille coche ensemble : son groupe, réduit à celles du registre, ou elle seule. */
function chipMembers(metric: string, available: string[]): string[] {
  const group = groupOf(metric)
  return group ? available.filter((m) => group.metrics.includes(m)) : [metric]
}

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

/**
 * Le choix gardé, réduit aux métriques connues et dans leur ordre. Jamais choisi (`null`) : toutes.
 * Vide : aucune, la vue invite à en choisir une. Des métriques toutes disparues du registre : toutes.
 * Un groupe dont une seule métrique est gardée (un choix d'avant les groupes) compte en entier.
 */
export function effectiveSelection(stored: string[] | null, available: string[]): string[] {
  if (stored === null) return available
  if (stored.length === 0) return []
  const kept = available.filter((metric) => stored.includes(metric))
  if (kept.length === 0) return available
  return available.filter((metric) => chipMembers(metric, available).some((m) => kept.includes(m)))
}

/** Ajoute ou retire la pastille d'une métrique (elle seule, ou tout son groupe), la dernière comprise. */
export function toggleMetric(current: string[], metric: string, available: string[]): string[] {
  const members = chipMembers(metric, available)
  if (members.some((m) => current.includes(m))) return current.filter((m) => !members.includes(m))
  return available.filter((m) => members.includes(m) || current.includes(m))
}

/** « Tout cocher » ou « Tout décocher ». */
export function setAll(available: string[], on: boolean): string[] {
  return on ? [...available] : []
}

function readStored(): string[] | null {
  try {
    const value: unknown = JSON.parse(window.localStorage.getItem(STORAGE_KEY) ?? 'null')
    return Array.isArray(value) && value.every((m) => typeof m === 'string') ? value : null
  } catch {
    return null
  }
}

function store(next: string[]) {
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(next))
  } catch {
    // Navigation privée : le choix vaut pour cette visite.
  }
}

/** Le choix commun au mois, à l'année et au jour. `null` : jamais choisi, donc toutes les métriques. */
export function useMetricSelection() {
  const [stored, setStored] = useState<string[] | null>(readStored)

  const toggle = useCallback((metric: string, available: string[]) => {
    setStored((current) => {
      const next = toggleMetric(effectiveSelection(current, available), metric, available)
      store(next)
      return next
    })
  }, [])

  const all = useCallback((available: string[], on: boolean) => {
    const next = setAll(available, on)
    store(next)
    setStored(next)
  }, [])

  return { stored, toggle, setAll: all }
}

export type MetricSelection = ReturnType<typeof useMetricSelection>
