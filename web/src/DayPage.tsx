import { useEffect, useRef, useState, type ReactNode } from 'react'
import { ACTIVITY_METRICS, ActivityCard } from './ActivityCard'
import { useActivity } from './useActivity'
import { LocationCard } from './LocationCard'
import { useLocations } from './useLocations'
import { monthName, monthOf } from './calendar'
import type { Op } from './reconcile'
import { isAdd } from './reconcile'
import { MetricFilters, NO_METRIC } from './MetricFilters'
import { effectiveSelection, metricShown } from './metrics'
import { musicAlert } from './music'
import { MusicCard } from './MusicCard'
import type { useMusic } from './useMusic'
import { useHealth, useKnownMetrics, usePrefetch } from './queries'
import { addDays, dayTitle, emptyLabel, longDate, parisInstant, parisTime } from './time'
import type { useCoffee } from './useCoffee'
import { useMonth } from './useMonth'
import { useGames } from './useGames'
import { useGithub } from './useGithub'
import { useTwitch } from './useTwitch'
import { useVideo } from './useVideo'
import { GamesCard } from './GamesCard'
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
  /** Le choix gardé, commun au mois, à l'année et au jour. */
  storedMetrics: string[] | null
  onToggleMetric: (metric: string, available: string[]) => void
  onSetAllMetrics: (available: string[], on: boolean) => void
  coffee: ReturnType<typeof useCoffee>
  music: ReturnType<typeof useMusic>
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
  storedMetrics,
  onToggleMetric,
  onSetAllMetrics,
  coffee,
  music,
}: Props) {
  const [pickerOpen, setPickerOpen] = useState(false)
  const [picked, setPicked] = useState('12:00')
  const [pickError, setPickError] = useState<string | null>(null)
  const timeInput = useRef<HTMLInputElement>(null)
  const cards = useMasonry<HTMLDivElement>()
  const isToday = date === today
  const { cups } = coffee
  // Les métriques de la dernière lecture d'un mois ou d'une année : les filtres sont là d'emblée, et une
  // carte décochée n'apparaît pas le temps que le mois se lise.
  const metrics = useKnownMetrics()
  const available = metrics.map((m) => m.metric)
  const selected = effectiveSelection(storedMetrics, available)
  const shows = metricShown(storedMetrics, available)
  const none = available.length > 0 && selected.length === 0
  const activityShown = ACTIVITY_METRICS.filter(shows)
  // La journée vue par le mois : ses erreurs, et ce que disent les cartes d'un jour sans données.
  const calendarDay = useMonth(monthOf(date)).month?.days.find((day) => day.date === date) ?? null
  const health = useHealth()
  const alert = shows('music.listen') ? musicAlert(health) : null
  // Chaque carte ne lit que si elle est affichée.
  const activity = useActivity(date, activityShown.length > 0)
  const video = useVideo(date, shows('video.watch'))
  const twitch = useTwitch(date, shows('stream.watch'))
  const games = useGames(date, shows('game.play'))
  const github = useGithub(date, shows('code.commit') || shows('code.pull_request'))
  const locations = useLocations(date, shows('place.visit'))
  const prefetch = usePrefetch()
  const previous = addDays(date, -1)
  const next = addDays(date, 1)

  // Les cafés de la veille et du lendemain se lisent d'avance ; les cartes, elles, attendent qu'on y aille.
  useEffect(() => {
    prefetch.day(previous)
    if (next <= today) prefetch.day(next)
  }, [previous, next, today, prefetch])

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
          <button type="button" className="round" aria-label="Jour précédent" onClick={() => select(previous)}>
            <Chevron d="M15 5l-7 7 7 7" />
          </button>
          <button type="button" className="round" aria-label="Jour suivant" disabled={isToday} onClick={() => select(next)}>
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

        {activityShown.length > 0 && <ActivityCard date={date} today={today} state={activity} health={health} shown={activityShown} />}
        {shows('music.listen') && <MusicCard date={date} today={today} state={music} health={health} />}
        {shows('video.watch') && <VideoCard date={date} today={today} state={video} health={health} />}
        {shows('stream.watch') && <TwitchCard date={date} today={today} state={twitch} health={health} calendarDay={calendarDay} />}
        {/* Ni les succès ni les parties n'ont de filtre : la carte « Jeux » les montre avec le temps de jeu. */}
        {shows('game.play') && <GamesCard date={date} today={today} state={games} health={health} calendarDay={calendarDay} />}
        {/* Reviews, issues et commentaires n'ont pas de filtre : la carte les montre avec les commits et les pull requests. */}
        {(shows('code.commit') || shows('code.pull_request')) && <GithubCard date={date} today={today} state={github} health={health} calendarDay={calendarDay} />}
        {/* La trace n'a pas de filtre (`location.point` est hors du calendrier) : elle suit les lieux. */}
        {shows('place.visit') && <LocationCard date={date} today={today} state={locations} health={health} />}
      </div>
    </main>
  )
}
