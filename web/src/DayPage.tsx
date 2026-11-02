import { useRef, useState } from 'react'
import type { MonthDay } from './api'
import { monthName, monthOf } from './calendar'
import type { Op } from './reconcile'
import { isAdd } from './reconcile'
import { musicAlert } from './music'
import { MusicCard } from './MusicCard'
import type { useMusic } from './useMusic'
import { addDays, dayTitle, emptyLabel, longDate, parisInstant, parisTime } from './time'
import type { useCoffee } from './useCoffee'

interface Props {
  date: string
  today: string
  onSelect: (date: string) => void
  onShowMonth: () => void
  /** La journée vue par le mois : ses erreurs. `null` tant que le mois n'est pas lu. */
  calendarDay: MonthDay | null
  coffee: ReturnType<typeof useCoffee>
  music: ReturnType<typeof useMusic>
  onLogout: () => void
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

export function DayPage({ date, today, onSelect, onShowMonth, calendarDay, coffee, music, onLogout }: Props) {
  const [pickerOpen, setPickerOpen] = useState(false)
  const [picked, setPicked] = useState('12:00')
  const [pickError, setPickError] = useState<string | null>(null)
  const timeInput = useRef<HTMLInputElement>(null)
  const isToday = date === today
  const { cups } = coffee
  const alert = musicAlert(music.health)

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
      setPickError('Choisissez une heure.')
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
      <button type="button" className="back" onClick={onShowMonth} aria-label={`Voir le mois : ${monthName(monthOf(date)).toLowerCase()}`}>
        <Chevron d="M15 5l-7 7 7 7" />
        {monthName(monthOf(date))}
      </button>
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

      <div className="cards">
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

      <MusicCard date={date} today={today} state={music} />
      </div>

      <button type="button" className="quiet" onClick={onLogout}>
        Se déconnecter
      </button>
    </main>
  )
}
