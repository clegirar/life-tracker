import { useState, type ReactNode } from 'react'
import type { Year } from './api'
import { monthGrid, monthName, monthOf } from './calendar'
import { Segments } from './Segments'
import { heatLevel, heatThresholds, monthSummary, yearMonths } from './year'

interface Props {
  /** Le bouton du tiroir, sur iPhone. */
  menu: ReactNode
  year: string
  today: string
  data: Year | null
  loadFailed: boolean
  onYear: (year: string) => void
  onOpenMonth: (month: string) => void
  onReload: () => void
}

function Chevron({ d }: { d: string }) {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.4" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d={d} />
    </svg>
  )
}

// La couleur de chaque métrique connue ; une métrique nouvelle prend la teinte neutre, sans code à changer.
const HEAT_CLASS: Record<string, string> = { coffee: 'heat-coffee', 'music.listen': 'heat-music' }
const STORAGE_KEY = 'year.metric'

/** Six semaines pour chaque mois : les douze mini-mois ont la même hauteur et s'alignent. */
function sixWeeks(month: string): (string | null)[] {
  const cells = monthGrid(month).flat()
  return [...cells, ...Array.from({ length: 42 - cells.length }, () => null)]
}

function storedMetric(): string | null {
  try {
    return window.localStorage.getItem(STORAGE_KEY)
  } catch {
    return null
  }
}

export function YearPage({ menu, year, today, data, loadFailed, onYear, onOpenMonth, onReload }: Props) {
  const [chosen, setChosen] = useState<string | null>(storedMetric)
  const metrics = data?.metrics ?? []
  const metric = metrics.find((m) => m.metric === chosen) ?? metrics[0]
  const shown = data && String(data.year) === year ? data : null
  const days = new Map((shown?.days ?? []).map((day) => [day.date, day]))
  const values = metric ? [...days.values()].filter((day) => day.date <= today).map((day) => day.values[metric.metric] ?? null) : []
  const thresholds = heatThresholds(values)
  const isCurrent = year === today.slice(0, 4)
  const current = monthOf(today)

  const choose = (next: string) => {
    setChosen(next)
    try {
      window.localStorage.setItem(STORAGE_KEY, next)
    } catch {
      // Navigation privée : le choix vaut pour cette visite.
    }
  }

  return (
    <main className="page year-page">
      {menu && <div className="topbar">{menu}</div>}
      <header className="header">
        <div className="header-text">
          <h1 className="title">{year}</h1>
        </div>
        <nav className="nav" aria-label="Changer d'année">
          <button type="button" className="round" aria-label="Année précédente" onClick={() => onYear(String(Number(year) - 1))}>
            <Chevron d="M15 5l-7 7 7 7" />
          </button>
          <button type="button" className="round" aria-label="Année suivante" disabled={isCurrent} onClick={() => onYear(String(Number(year) + 1))}>
            <Chevron d="M9 5l7 7-7 7" />
          </button>
        </nav>
      </header>

      {loadFailed && (
        <div className="notice" role="alert">
          <span>Impossible d'actualiser cette année.</span>
          <button type="button" className="notice-action" onClick={onReload}>
            Réessayer
          </button>
        </div>
      )}

      {metrics.length > 1 && metric && (
        <Segments label="Métrique affichée" options={metrics.map((m) => [m.metric, m.label] as const)} value={metric.metric} onChange={choose} />
      )}

      <ol className={`year-grid ${metric ? (HEAT_CLASS[metric.metric] ?? '') : ''}`} data-testid="year-grid" data-metric={metric?.metric}>
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
                    const value = metric ? (days.get(date)?.values[metric.metric] ?? null) : null
                    return (
                      <span
                        key={date}
                        className="dot"
                        data-date={date}
                        data-heat={shown ? heatLevel(value, thresholds) : undefined}
                        data-today={date === today ? '' : undefined}
                      />
                    )
                  })}
                </span>
                {shown && (
                  <span id={summaryId} hidden>
                    {monthSummary(month, (shown.days ?? []).filter((day) => monthOf(day.date) === month), metric, today)}
                  </span>
                )}
              </button>
            </li>
          )
        })}
      </ol>

      <section className={`legend ${metric ? (HEAT_CLASS[metric.metric] ?? '') : ''}`} aria-label="Légende">
        <ul className="legend-list">
          <li>
            <span className="dot legend-dot" data-heat="none" aria-hidden="true" />
            Pas de données
          </li>
          <li>
            <span className="dot legend-dot" data-heat="0" aria-hidden="true" />
            Rien
          </li>
          <li>
            Moins
            {([1, 2, 3, 4] as const).map((level) => (
              <span key={level} className="dot legend-dot" data-heat={level} aria-hidden="true" />
            ))}
            Plus
          </li>
        </ul>
        <p className="legend-note">Vide : pas de données. Pâle : rien ce jour-là.</p>
      </section>
    </main>
  )
}
