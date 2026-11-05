import { useCallback, useEffect, useRef, useState } from 'react'
import { flushSync } from 'react-dom'
import { fetchDay, logout, Unauthorized } from './api'
import { monthOf } from './calendar'
import { DayPage } from './DayPage'
import { Login } from './Login'
import { useMetricSelection } from './metrics'
import { MonthPage } from './MonthPage'
import { STATUS, TODAY, routePath, useRoute, type Route } from './route'
import { MenuButton, Sidebar, useWide } from './Sidebar'
import { needsAction, sourceCard } from './status'
import { StatusPage } from './StatusPage'
import { parisDate } from './time'
import { useCoffee } from './useCoffee'
import { useMonth } from './useMonth'
import { useActivity } from './useActivity'
import { useMusic } from './useMusic'
import { useSourcesStatus } from './useSourcesStatus'
import { useYear } from './useYear'
import { YearPage } from './YearPage'

type Auth = 'unknown' | 'in' | 'out' | 'expired'

export function App() {
  const [auth, setAuth] = useState<Auth>('unknown')
  const [now, setNow] = useState(() => Date.now())
  const today = parisDate(now)
  const { route, navigate } = useRoute(today)
  // Le dernier jour ouvert : le mois le montre sélectionné, les données du jour restent les siennes.
  // `null` : aujourd'hui.
  const [lastDay, setLastDay] = useState<string | null>(null)
  const date = (route.page === 'day' ? route.date : lastDay) ?? today
  const wide = useWide()
  // Les métriques affichées : un seul choix pour le mois et l'année.
  const metricSelection = useMetricSelection()
  const [menuOpen, setMenuOpen] = useState(false)
  const menuButton = useRef<HTMLButtonElement>(null)

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

  const shownMonth = route.page === 'month' ? route.month : null
  // Vit au-dessus de l'écran de connexion : une écriture refusée par une session expirée y survit.
  const coffee = useCoffee(date, onSessionLost, auth === 'in')
  const music = useMusic(date, onSessionLost, auth === 'in')
  const activity = useActivity(date, onSessionLost, auth === 'in')
  // Le mois affiché, ou celui du jour : la page du jour y lit si sa journée est incomplète.
  const calendar = useMonth(shownMonth ?? monthOf(date), onSessionLost, auth === 'in', shownMonth)
  // L'année n'est lue que vue ouverte : la rouvrir la relit.
  const shownYear = route.page === 'year' ? route.year : today.slice(0, 4)
  const year = useYear(shownYear, onSessionLost, auth === 'in' && route.page === 'year')
  // Lu dès la connexion : le point du menu en dépend. Relu à chaque ouverture de la page.
  const sources = useSourcesStatus(onSessionLost, auth === 'in', route.page === 'status')
  const attention = (sources.status?.sources ?? []).some((source) => needsAction(sourceCard(source, now).state))

  // Une panne n'est pas une déconnexion : la page du jour saura la signaler.
  useEffect(() => {
    fetchDay(parisDate(Date.now())).then(
      () => setAuth('in'),
      (error) => setAuth(error instanceof Unauthorized ? 'out' : 'in'),
    )
  }, [])

  const closeMenu = useCallback(() => {
    // Le focus revient au bouton qui a ouvert le tiroir, une fois la page sortie de `inert`.
    flushSync(() => setMenuOpen(false))
    menuButton.current?.focus({ preventScroll: true })
  }, [])

  if (auth === 'unknown') return null
  if (auth !== 'in') {
    return <Login expired={auth === 'expired'} onLoggedIn={() => setAuth('in')} />
  }

  const go = (next: Route) => {
    if (route.page === 'day') setLastDay(route.date)
    if (next.page === 'day') setLastDay(next.date)
    setMenuOpen(false)
    navigate(next)
  }
  const openDay = (next: string) => go({ page: 'day', date: next === today ? null : next })
  const drawerOpen = !wide && menuOpen
  const menu = wide ? null : <MenuButton open={drawerOpen} onOpen={() => setMenuOpen(true)} buttonRef={menuButton} attention={attention} />

  let page
  if (route.page === 'status') {
    page = (
      <StatusPage
        menu={menu}
        now={now}
        data={sources.status}
        loadFailed={sources.loadFailed}
        onReload={() => void sources.reload()}
      />
    )
  } else if (route.page === 'year') {
    page = (
      <YearPage
        menu={menu}
        year={route.year}
        today={today}
        data={year.year}
        loadFailed={year.loadFailed}
        onYear={(next) => go({ page: 'year', year: next })}
        onOpenMonth={(month) => go({ page: 'month', month })}
        onReload={() => void year.reload()}
        storedMetrics={metricSelection.stored}
        onToggleMetric={metricSelection.toggle}
      />
    )
  } else if (route.page === 'month') {
    page = (
      <MonthPage
        menu={menu}
        onShowYear={() => go({ page: 'year', year: route.month.slice(0, 4) })}
        month={route.month}
        today={today}
        selected={date}
        data={calendar.month}
        loadFailed={calendar.loadFailed}
        health={music.health}
        onMonth={(month) => go({ page: 'month', month })}
        onReload={() => void calendar.reload()}
        onOpenDay={openDay}
        storedMetrics={metricSelection.stored}
        onToggleMetric={metricSelection.toggle}
      />
    )
  } else {
    page = (
      <DayPage
        menu={menu}
        date={date}
        today={today}
        onSelect={openDay}
        onShowMonth={() => go({ page: 'month', month: monthOf(date) })}
        calendarDay={calendar.month?.days.find((day) => day.date === date) ?? null}
        coffee={coffee}
        music={music}
        activity={activity}
      />
    )
  }

  return (
    <div className="shell">
      <Sidebar
        wide={wide}
        open={drawerOpen}
        onClose={closeMenu}
        current={route.page === 'status' ? 'status' : 'calendar'}
        calendarHref={routePath(TODAY)}
        onCalendar={() => go(TODAY)}
        statusHref={routePath(STATUS)}
        onStatus={() => go(STATUS)}
        attention={attention}
        onLogout={async () => {
          await logout().catch(() => undefined)
          coffee.reset()
          music.reset()
          activity.reset()
          calendar.reset()
          year.reset()
          sources.reset()
          setLastDay(null)
          setMenuOpen(false)
          navigate(TODAY, true)
          setAuth('out')
        }}
      />
      {/* `inert` : le tiroir ouvert est seul à recevoir le focus et les touchers. */}
      <div className="content" inert={drawerOpen}>
        {page}
      </div>
    </div>
  )
}
