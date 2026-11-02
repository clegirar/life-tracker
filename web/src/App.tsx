import { useCallback, useEffect, useState } from 'react'
import { fetchDay, logout, Unauthorized } from './api'
import { monthOf } from './calendar'
import { DayPage } from './DayPage'
import { Login } from './Login'
import { MonthPage } from './MonthPage'
import { parisDate } from './time'
import { useCoffee } from './useCoffee'
import { useMonth } from './useMonth'
import { useMusic } from './useMusic'

type Auth = 'unknown' | 'in' | 'out' | 'expired'

export function App() {
  const [auth, setAuth] = useState<Auth>('unknown')
  // `null` : suivre aujourd'hui, y compris quand minuit passe avec la page ouverte.
  const [selected, setSelected] = useState<string | null>(null)
  const [now, setNow] = useState(() => Date.now())
  const today = parisDate(now)
  const date = selected ?? today
  // `null` : la page du jour ; sinon le mois affiché.
  const [shownMonth, setShownMonth] = useState<string | null>(null)

  useEffect(() => {
    const tick = () => setNow(Date.now())
    const timer = setInterval(tick, 10_000)
    document.addEventListener('visibilitychange', tick)
    return () => {
      clearInterval(timer)
      document.removeEventListener('visibilitychange', tick)
    }
  }, [])

  const onSessionLost = useCallback(() => setAuth('expired'), [])

  // Vit au-dessus de l'écran de connexion : une écriture refusée par une session expirée y survit.
  const coffee = useCoffee(date, onSessionLost, auth === 'in')
  const music = useMusic(date, onSessionLost, auth === 'in')
  // Le mois affiché, ou celui du jour : la page du jour y lit si sa journée est incomplète.
  const calendar = useMonth(shownMonth ?? monthOf(date), onSessionLost, auth === 'in', shownMonth)

  // Une panne n'est pas une déconnexion : la page du jour saura la signaler.
  useEffect(() => {
    fetchDay(parisDate(Date.now())).then(
      () => setAuth('in'),
      (error) => setAuth(error instanceof Unauthorized ? 'out' : 'in'),
    )
  }, [])

  if (auth === 'unknown') return null
  if (auth !== 'in') {
    return <Login expired={auth === 'expired'} onLoggedIn={() => setAuth('in')} />
  }
  const select = (next: string) => setSelected(next === today ? null : next)
  if (shownMonth) {
    return (
      <MonthPage
        month={shownMonth}
        today={today}
        selected={date}
        data={calendar.month}
        loadFailed={calendar.loadFailed}
        health={music.health}
        onMonth={setShownMonth}
        onReload={() => void calendar.reload()}
        onOpenDay={(next) => {
          select(next)
          setShownMonth(null)
        }}
      />
    )
  }
  return (
    <DayPage
      date={date}
      today={today}
      onSelect={select}
      onShowMonth={() => setShownMonth(monthOf(date))}
      calendarDay={calendar.month?.days.find((day) => day.date === date) ?? null}
      coffee={coffee}
      music={music}
      onLogout={async () => {
        await logout().catch(() => undefined)
        coffee.reset()
        music.reset()
        calendar.reset()
        setSelected(null)
        setShownMonth(null)
        setAuth('out')
      }}
    />
  )
}
