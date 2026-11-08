import { useRef, useState, type ReactNode } from 'react'
import type { MonthDay, MonthMetric } from './api'
import { ACTIVITY_METRICS, ActivityCard } from './ActivityCard'
import type { useActivity } from './useActivity'
import { LocationCard } from './LocationCard'
import type { useLocations } from './useLocations'
import { monthName, monthOf } from './calendar'
import type { Op } from './reconcile'
import { isAdd } from './reconcile'
import { MetricFilters, NO_METRIC } from './MetricFilters'
import { effectiveSelection } from './metrics'
import { musicAlert } from './music'
import { MusicCard } from './MusicCard'
import type { useMusic } from './useMusic'
import { addDays, dayTitle, emptyLabel, longDate, parisInstant, parisTime } from './time'
import type { useCoffee } from './useCoffee'
import type { useSteam } from './useSteam'
import type { useGithub } from './useGithub'
import type { useTwitch } from './useTwitch'
import type { useVideo } from './useVideo'
import { SteamCard } from './SteamCard'
import { GithubCard } from './GithubCard'
import { useMasonry } from './useMasonry'
import { TwitchCard } from './TwitchCard'
import { VideoCard } from './VideoCard'

interface Props {
  /** Le bouton du tiroir, sur iPhone. */
  menu: ReactNode
  date: string
  today: string
  onSelect: (date: string) => void
  onShowMonth: () => void
  /** La journée vue par le mois : ses erreurs. `null` tant que le mois n'est pas lu. */
  calendarDay: MonthDay | null
  /** Les métriques du registre, lues avec le mois ; vide tant qu'il n'est pas lu. */
  metrics: MonthMetric[]
  /** Le choix gardé, commun au mois, à l'année et au jour. */
  storedMetrics: string[] | null
  onToggleMetric: (metric: string, available: string[]) => void
  onSetAllMetrics: (available: string[], on: boolean) => void
  coffee: ReturnType<typeof useCoffee>
  music: ReturnType<typeof useMusic>
  activity: ReturnType<typeof useActivity>
  video: ReturnType<typeof useVideo>
  twitch: ReturnType<typeof useTwitch>
  steam: ReturnType<typeof useSteam>
  github: ReturnType<typeof useGithub>
  locations: ReturnType<typeof useLocations>
}

function Chevron({ d }: { d: string }) {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.4" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d={d} />
    </svg>
  )
}

function failureText(op: Op, date: string, today: string): string {
  const what = isAdd(op) ? 'Le café' : "L'annulation du café"
  const e = isAdd(op) ? '' : 'e'
  // L'échec peut concerner un autre jour que celui affiché.
  const when = op.date === date ? op.time : `${op.time}, ${longDate(op.date, today).toLowerCase()},`
  return op.final
    ? `${what} de ${when} a été refusé${e} par le serveur.`
    : `${what} de ${when} n'a pas été enregistré${e}.`
}

export function DayPage({
  menu,
  date,
  today,
  onSelect,
  onShowMonth,
  calendarDay,
  metrics,
  storedMetrics,
  onToggleMetric,
  onSetAllMetrics,
  coffee,
  music,
  activity,
  video,
  twitch,
  steam,
  github,
  locations,
}: Props) {
  const [pickerOpen, setPickerOpen] = useState(false)
  const [picked, setPicked] = useState('12:00')
  const [pickError, setPickError] = useState<string | null>(null)
  const timeInput = useRef<HTMLInputElement>(null)
  const cards = useMasonry<HTMLDivElement>()
  const isToday = date === today
  const { cups } = coffee
  const available = metrics.map((m) => m.metric)
  const selected = effectiveSelection(storedMetrics, available)
  // Une métrique que les filtres ne connaissent pas (le mois n'est pas encore lu) reste affichée.
  const shows = (metric: string) => !available.includes(metric) || selected.includes(metric)
  const none = available.length > 0 && selected.length === 0
  const activityShown = ACTIVITY_METRICS.filter(shows)
  const alert = shows('music.listen') ? musicAlert(music.health) : null

  function select(next: string) {
    setPickerOpen(false)
    setPickError(null)
    onSelect(next)
  }

  function openPicker() {
    setPickError(null)
    setPicked(isToday ? parisTime(Date.now()) : '12:00')
    setPickerOpen(true)
    // Le champ n'existe qu'au prochain rendu.
    requestAnimationFrame(() => timeInput.current?.focus())
  }

  function addPicked() {
    if (!picked) {
      setPickError('Choisis une heure.')
      return
    }
    const instant = parisInstant(date, picked)
    if (instant === null) {
      setPickError("Cette heure n'existe pas ce jour-là, à cause du changement d'heure.")
      return
    }
    if (instant > Date.now()) {
      setPickError("Cette heure n'est pas encore passée.")
      return
    }
    setPickError(null)
    setPickerOpen(false)
    coffee.addCup(instant)
  }

  return (
    <main className="page">
      <div className="topbar">
        {menu}
        <button type="button" className="back" onClick={onShowMonth} aria-label={`Voir le mois : ${monthName(monthOf(date)).toLowerCase()}`}>
          <Chevron d="M15 5l-7 7 7 7" />
          {monthName(monthOf(date))}
        </button>
      </div>
      <header className="header">
        <div className="header-text">
          <p className="subtitle">{longDate(date, today)}</p>
          <h1 className="title">{dayTitle(date, today)}</h1>
        </div>
        <nav className="nav" aria-label="Changer de jour">
          <button type="button" className="round" aria-label="Jour précédent" onClick={() => select(addDays(date, -1))}>
            <Chevron d="M15 5l-7 7 7 7" />
          </button>
          <button type="button" className="round" aria-label="Jour suivant" disabled={isToday} onClick={() => select(addDays(date, 1))}>
            <Chevron d="M9 5l7 7-7 7" />
          </button>
        </nav>
      </header>

      <MetricFilters
        metrics={metrics}
        selected={selected}
        onToggle={(metric) => onToggleMetric(metric, available)}
        onSetAll={(on) => onSetAllMetrics(available, on)}
      />

      {coffee.loadFailed && (
        <div className="notice" role="alert">
          <span>Impossible d'actualiser cette journée.</span>
          <button type="button" className="notice-action" onClick={() => void coffee.reload()}>
            Réessayer
          </button>
        </div>
      )}
      {coffee.failed.map((op) => (
        <div className="notice" role="alert" key={op.record.key}>
          <span>{failureText(op, date, today)}</span>
          <span className="notice-actions">
            {!op.final && (
              <button type="button" className="notice-action" onClick={() => coffee.retry(op)}>
                Réessayer
              </button>
            )}
            <button type="button" className="notice-action" onClick={() => coffee.dismiss(op)}>
              Ignorer
            </button>
          </span>
        </div>
      ))}
      {coffee.slow && (
        <p className="notice" role="status">
          C'est bien enregistré. Le décompte sera confirmé dès que le serveur l'aura traité.
        </p>
      )}

      {calendarDay && calendarDay.errors > 0 && (
        <p className="notice notice-block" role="status" data-testid="day-incomplete">
          <span>
            <strong>Journée incomplète.</strong>{' '}
            {calendarDay.errors > 1
              ? `${calendarDay.errors} enregistrements de ce jour sont en erreur : gardés au journal, ils ne sont pas encore comptés.`
              : 'Un enregistrement de ce jour est en erreur : gardé au journal, il n’est pas encore compté.'}
          </span>
        </p>
      )}

      {alert && (
        <p className="notice notice-block" role="status" data-testid="music-alert">
          <span>
            <strong>{alert.title}</strong> {alert.text}
          </span>
        </p>
      )}

      {none && (
        <p className="no-metric" data-testid="no-metric">
          {NO_METRIC}
        </p>
      )}

      <div ref={cards} className="cards">
        {shows('coffee') && (
          <section className="card card-coffee" aria-labelledby="titre-cafe">
            <h2 id="titre-cafe" className="card-title">
              Café
            </h2>
            <div className="count-row">
              <p className="count" data-testid="count">
                <span className="count-value">{cups ? cups.length : '–'}</span>
                <span className="count-label">{cups && cups.length > 1 ? 'cafés' : 'café'}</span>
              </p>
              <button
                type="button"
                className="add"
                aria-label={isToday ? 'Ajouter un café maintenant' : 'Ajouter un café à une autre heure'}
                onClick={() => (isToday ? coffee.addCup(Date.now()) : openPicker())}
              >
                <svg width="26" height="26" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.6" strokeLinecap="round" aria-hidden="true">
                  <path d="M12 5v14M5 12h14" />
                </svg>
              </button>
            </div>

            {cups?.length === 0 && <p className="empty">{emptyLabel(date, today)}</p>}
            <ul className="cups">
              {cups?.map((cup) => (
                <li key={`${cup.source}/${cup.key}`} className={coffee.isLocal(cup.key) ? 'cup cup-new' : 'cup'}>
                  <span className="cup-time">{cup.time}</span>
                  <button type="button" className="cup-cancel" aria-label={`Annuler le café de ${cup.time}`} onClick={() => coffee.cancelCup(cup)}>
                    Annuler
                  </button>
                </li>
              ))}
            </ul>

            {pickerOpen && (
              <div className="picker">
                <div className="picker-row">
                  <label className="field">
                    Heure du café
                    <input ref={timeInput} type="time" value={picked} onChange={(e) => { setPicked(e.target.value); setPickError(null) }} />
                  </label>
                  <button type="button" className="primary" onClick={addPicked}>
                    Ajouter
                  </button>
                </div>
                {pickError && (
                  <p className="field-error" role="alert">
                    {pickError}
                  </p>
                )}
              </div>
            )}
            <button type="button" className="link" onClick={() => (pickerOpen ? setPickerOpen(false) : openPicker())}>
              {pickerOpen ? 'Fermer' : 'Noter à une autre heure'}
            </button>
          </section>
        )}

        {activityShown.length > 0 && <ActivityCard date={date} today={today} state={activity} health={music.health} shown={activityShown} />}
        {shows('music.listen') && <MusicCard date={date} today={today} state={music} />}
        {shows('video.watch') && <VideoCard date={date} today={today} state={video} health={music.health} />}
        {shows('stream.watch') && <TwitchCard date={date} today={today} state={twitch} health={music.health} calendarDay={calendarDay} />}
        {/* Les succès n'ont pas de filtre : la carte les montre avec le temps de jeu. */}
        {shows('game.play') && <SteamCard date={date} today={today} state={steam} health={music.health} calendarDay={calendarDay} />}
        {/* Reviews, issues et commentaires n'ont pas de filtre : la carte les montre avec les commits et les pull requests. */}
        {(shows('code.commit') || shows('code.pull_request')) && <GithubCard date={date} today={today} state={github} health={music.health} calendarDay={calendarDay} />}
        {/* La trace n'a pas de filtre (`location.point` est hors du calendrier) : elle suit les lieux. */}
        {shows('place.visit') && <LocationCard date={date} today={today} state={locations} health={music.health} />}
      </div>
    </main>
  )
}
