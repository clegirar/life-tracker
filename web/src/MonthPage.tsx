import type { ReactNode } from 'react'
import type { Month, SourcesHealth } from './api'
import { addMonths, cellLabel, compact, errorCountText, marker, MARKER_LABELS, monthGrid, monthName, monthOf, WEEKDAYS, type Marker } from './calendar'
import { musicAlert } from './music'

interface Props {
  /** Le bouton du tiroir, sur iPhone. */
  menu: ReactNode
  month: string
  today: string
  selected: string
  data: Month | null
  loadFailed: boolean
  health: SourcesHealth | null
  onMonth: (month: string) => void
  onOpenDay: (date: string) => void
  onReload: () => void
}

function Chevron({ d }: { d: string }) {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.4" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d={d} />
    </svg>
  )
}

// Une couleur par métrique connue ; une métrique nouvelle prend la couleur neutre, sans code à changer.
const METRIC_CLASS: Record<string, string> = { coffee: 'v-coffee', 'music.listen': 'v-music' }

export function MonthPage({ menu, month, today, selected, data, loadFailed, health, onMonth, onOpenDay, onReload }: Props) {
  const isCurrent = month === monthOf(today)
  const days = new Map((data?.month === month ? data.days : []).map((day) => [day.date, day]))
  const metrics = data?.metrics ?? []
  const present = new Set<Marker>()
  for (const day of days.values()) {
    const m = marker(day)
    if (m) present.add(m)
  }
  const alert = musicAlert(health)
  const errors = health?.errors ?? null

  return (
    <main className="page month-page">
      {menu && <div className="topbar">{menu}</div>}
      <header className="header">
        <div className="header-text">
          <p className="subtitle">{month.slice(0, 4)}</p>
          <h1 className="title">{monthName(month)}</h1>
        </div>
        <nav className="nav" aria-label="Changer de mois">
          <button type="button" className="round" aria-label="Mois précédent" onClick={() => onMonth(addMonths(month, -1))}>
            <Chevron d="M15 5l-7 7 7 7" />
          </button>
          <button type="button" className="round" aria-label="Mois suivant" disabled={isCurrent} onClick={() => onMonth(addMonths(month, 1))}>
            <Chevron d="M9 5l7 7-7 7" />
          </button>
        </nav>
      </header>

      {loadFailed && (
        <div className="notice" role="alert">
          <span>Impossible d'actualiser ce mois.</span>
          <button type="button" className="notice-action" onClick={onReload}>
            Réessayer
          </button>
        </div>
      )}
      {alert && (
        <p className="notice notice-block" role="status">
          <span>
            <strong>{alert.title}</strong> {alert.text}
          </span>
        </p>
      )}
      {errors !== null && (
        <p className={errors > 0 ? 'notice' : 'status-quiet'} role="status" data-testid="error-count">
          <span>
            {errors > 0 ? <strong>{errorCountText(errors)}.</strong> : errorCountText(errors)}
            {errors > 0 && ' Gardés au journal, ils attendent une correction ; leurs journées sont marquées incomplètes.'}
          </span>
        </p>
      )}

      <section className="card month-card" aria-label={`${monthName(month)} ${month.slice(0, 4)}`}>
        <div className="weekdays" aria-hidden="true">
          {WEEKDAYS.map(([short, long]) => (
            <span key={long}>{short}</span>
          ))}
        </div>
        <ol className="month-grid" data-testid="month-grid">
          {monthGrid(month)
            .flat()
            .map((date, index) => {
              if (!date) return <li key={`vide-${index}`} className="cell-empty" aria-hidden="true" />
              const day = days.get(date)
              const future = date > today
              const mark = day ? marker(day) : null
              return (
                <li key={date}>
                  <button
                    type="button"
                    className="cell"
                    data-date={date}
                    data-marker={mark ?? undefined}
                    aria-current={date === today ? 'date' : undefined}
                    aria-pressed={date === selected}
                    aria-label={day && data ? cellLabel(day, metrics, today) : date}
                    disabled={future}
                    onClick={() => onOpenDay(date)}
                  >
                    <span className="cell-number">{Number(date.slice(8))}</span>
                    <span className="cell-values" aria-hidden="true">
                      {!future &&
                        metrics.map((metric) => {
                          const value = day?.values[metric.metric] ?? null
                          return (
                            <span key={metric.metric} className={`cell-value ${METRIC_CLASS[metric.metric] ?? ''} ${value === 0 ? 'is-zero' : ''}`}>
                              {value === null ? ' ' : compact(value)}
                            </span>
                          )
                        })}
                    </span>
                    {mark && <span className={`mark mark-${mark}`} aria-hidden="true" />}
                  </button>
                </li>
              )
            })}
        </ol>
      </section>

      <section className="legend" aria-label="Légende">
        <ul className="legend-list">
          {metrics.map((metric) => (
            <li key={metric.metric}>
              <span className={`legend-swatch ${METRIC_CLASS[metric.metric] ?? ''}`} aria-hidden="true" />
              {metric.label}
            </li>
          ))}
          {(['incomplete', 'pending', 'check'] as const)
            .filter((m) => present.has(m))
            .map((m) => (
              <li key={m}>
                <span className={`mark mark-${m} mark-inline`} aria-hidden="true" />
                {MARKER_LABELS[m]}
              </li>
            ))}
        </ul>
        <p className="legend-note">Sans chiffre : pas de données. 0 : rien ce jour-là.</p>
      </section>
    </main>
  )
}
