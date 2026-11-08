import { useEffect, type ReactNode } from 'react'
import { monthGrid, monthName, monthOf } from './calendar'
import { DayDots, MetricsLegend } from './DayDots'
import { HeatLegend, MetricFilters } from './MetricFilters'
import { dayActivity, effectiveSelection, metricTone } from './metrics'
import { useKnownMetrics, usePrefetch } from './queries'
import { useReveal } from './useReveal'
import { useYear } from './useYear'
import { heatLevel, monthSummary, periodThresholds, yearMonths } from './year'
import { Spinner } from './Spinner'

interface Props {
  /** Le bouton du tiroir, sur iPhone. */
  menu: ReactNode
  year: string
  today: string
  onYear: (year: string) => void
  onOpenMonth: (month: string) => void
  /** Le choix gardé, commun au mois, à l'année et au jour. */
  storedMetrics: string[] | null
  onToggleMetric: (metric: string, available: string[]) => void
  onSetAllMetrics: (available: string[], on: boolean) => void
}

function Chevron({ d }: { d: string }) {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.4" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d={d} />
    </svg>
  )
}

/** Six semaines pour chaque mois : les douze mini-mois ont la même hauteur et s'alignent. */
function sixWeeks(month: string): (string | null)[] {
  const cells = monthGrid(month).flat()
  return [...cells, ...Array.from({ length: 42 - cells.length }, () => null)]
}

export function YearPage({ menu, year, today, onYear, onOpenMonth, storedMetrics, onToggleMetric, onSetAllMetrics }: Props) {
  const { year: shown, loadFailed, reload } = useYear(year)
  const prefetch = usePrefetch()
  const pending = shown === null
  const reveal = useReveal(year, !pending)
  // Les métriques de la dernière lecture, de cette année ou d'une autre : les filtres, la légende et le
  // type de case (intensité ou points) ne changent pas à l'arrivée des valeurs.
  const metrics = useKnownMetrics()
  const available = metrics.map((m) => m.metric)
  const selected = effectiveSelection(storedMetrics, available)
  const chosen = metrics.filter((m) => selected.includes(m.metric))
  const single = chosen.length === 1 ? chosen[0].metric : null
  // Tout décoché : ni intensité ni point, et l'invitation à la place de la légende.
  const none = available.length > 0 && selected.length === 0
  const days = new Map((shown?.days ?? []).map((day) => [day.date, day]))
  // Une métrique : l'intensité, par quartiles de l'année. Plusieurs : des points, comme dans le mois.
  const thresholds = single ? periodThresholds(shown?.days ?? [], single, today) : []
  const isCurrent = year === today.slice(0, 4)
  const current = monthOf(today)
  const previous = String(Number(year) - 1)
  const next = String(Number(year) + 1)

  // L'année lue, ses voisines se lisent d'avance (la suivante seulement si elle a commencé).
  useEffect(() => {
    if (pending) return
    prefetch.year(previous)
    if (!isCurrent) prefetch.year(next)
  }, [pending, previous, next, isCurrent, prefetch])

  return (
    <main className="page year-page">
      {menu && <div className="topbar">{menu}</div>}
      <header className="header">
        <div className="header-text">
          <h1 className="title">
            {year}
            {pending && <Spinner />}
          </h1>
        </div>
        <nav className="nav" aria-label="Changer d'année">
          <button
            type="button"
            className="round"
            aria-label="Année précédente"
            onPointerEnter={() => prefetch.year(previous)}
            onPointerDown={() => prefetch.year(previous)}
            onClick={() => onYear(previous)}
          >
            <Chevron d="M15 5l-7 7 7 7" />
          </button>
          <button
            type="button"
            className="round"
            aria-label="Année suivante"
            disabled={isCurrent}
            onPointerEnter={() => prefetch.year(next)}
            onPointerDown={() => prefetch.year(next)}
            onClick={() => onYear(next)}
          >
            <Chevron d="M9 5l7 7-7 7" />
          </button>
        </nav>
      </header>

      {loadFailed && (
        <div className="notice" role="alert">
          <span>Impossible d'actualiser cette année.</span>
          <button type="button" className="notice-action" onClick={() => void reload()}>
            Réessayer
          </button>
        </div>
      )}

      <MetricFilters metrics={metrics} selected={selected} onToggle={(metric) => onToggleMetric(metric, available)} onSetAll={(on) => onSetAllMetrics(available, on)} />

      <ol
        className={`year-grid ${single ? metricTone(single) : 'is-dots'}`}
        data-testid="year-grid"
        data-mode={none ? 'aucune' : single ? 'une' : 'plusieurs'}
        data-metrics={selected.join(' ')}
        aria-busy={pending}
        data-reveal={reveal ? '' : undefined}
      >
        {yearMonths(year).map((month) => {
          const future = month > current
          const summaryId = `resume-${month}`
          return (
            <li key={month}>
              <button
                type="button"
                className="mini"
                data-month={month}
                aria-label={`${monthName(month)} ${year}`}
                aria-describedby={shown ? summaryId : undefined}
                aria-current={month === current ? 'date' : undefined}
                disabled={future}
                onClick={() => onOpenMonth(month)}
              >
                <span className="mini-name">{monthName(month)}</span>
                <span className="mini-grid" aria-hidden="true">
                  {sixWeeks(month).map((date, index) => {
                    // Les cases d'un autre mois et les jours futurs restent vides : rien n'y est encore arrivé.
                    if (!date || date > today) return <span key={`vide-${index}`} className="dot-slot" />
                    const day = days.get(date)
                    if (!single) {
                      const activity = shown && !none ? dayActivity(day, selected) : null
                      return (
                        <span
                          key={date}
                          className="mini-day"
                          data-date={date}
                          data-today={date === today ? '' : undefined}
                          data-nodata={activity?.noData ? '' : undefined}
                        >
                          <span className="mini-day-number">{Number(date.slice(8))}</span>
                          <DayDots active={activity?.active ?? []} size="year" />
                        </span>
                      )
                    }
                    return (
                      <span
                        key={date}
                        className="dot"
                        data-date={date}
                        data-heat={shown ? heatLevel(day?.values[single] ?? null, thresholds) : undefined}
                        data-today={date === today ? '' : undefined}
                      />
                    )
                  })}
                </span>
                {shown && (
                  <span id={summaryId} hidden>
                    {monthSummary(month, shown.days.filter((day) => monthOf(day.date) === month), chosen, today)}
                  </span>
                )}
              </button>
            </li>
          )
        })}
      </ol>

      {none ? <MetricsLegend metrics={[]} mode="none" /> : single ? <HeatLegend metric={single} /> : chosen.length > 0 && <MetricsLegend metrics={chosen} mode="dots" />}
    </main>
  )
}
