import { useQueryClient } from '@tanstack/react-query'
import { useCallback, useEffect, useRef, useState } from 'react'
import { flushSync } from 'react-dom'
import { logout, Unauthorized } from './api'
import { AuthorizationPage } from './AuthorizationPage'
import { monthOf } from './calendar'
import { DayPage } from './DayPage'
import { Login } from './Login'
import { metricShown, useMetricSelection } from './metrics'
import { MonthPage } from './MonthPage'
import { dayQuery, onSessionLost as onQuerySessionLost, useKnownMetrics } from './queries'
import { SOURCES, STATUS, TODAY, routePath, useRoute, type Route } from './route'
import { MenuButton, Sidebar, useWide } from './Sidebar'
import { needsAction, sourceCard } from './status'
import { SourcesPage } from './SourcesPage'
import { StatusPage } from './StatusPage'
import { parisDate } from './time'
import { useCoffee } from './useCoffee'
import { useMusic } from './useMusic'
import { useSourcesStatus } from './useSourcesStatus'
import { YearPage } from './YearPage'

type Auth = 'unknown' | 'in' | 'out' | 'expired'

export function App() {
  const queryClient = useQueryClient()
  const [auth, setAuth] = useState<Auth>('unknown')
  const [now, setNow] = useState(() => Date.now())
  const today = parisDate(now)
  const { route, navigate } = useRoute(today)
  // Le dernier jour ouvert : le mois le montre sélectionné, les données du jour restent les siennes.
  // `null` : aujourd'hui.
  const [lastDay, setLastDay] = useState<string | null>(null)
  const date = (route.page === 'day' ? route.date : lastDay) ?? today
  const wide = useWide()
  // Les métriques affichées : un seul choix pour le mois, l'année et le jour.
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

  // Une session perdue en cours de route ; avant la sonde de connexion, rien à perdre.
  const onSessionLost = useCallback(() => setAuth((current) => (current === 'in' ? 'expired' : current)), [])
  // Toute lecture refusée pour session perdue le dit ici.
  useEffect(() => onQuerySessionLost(onSessionLost), [onSessionLost])

  // Les lectures du jour ne tournent que sur sa page, et seulement pour les cartes affichées. Le café et
  // la musique vivent ici, au-dessus de l'écran de connexion : une écriture refusée par une session
  // expirée y survit. Les autres cartes, le mois et l'année lisent depuis leur page.
  const shows = metricShown(metricSelection.stored, useKnownMetrics().map((m) => m.metric))
  const onDay = auth === 'in' && route.page === 'day'
  const coffee = useCoffee(date, onSessionLost, onDay && shows('coffee'))
  const music = useMusic(date, onSessionLost, onDay && shows('music.listen'))
  // Lu dès la connexion : le point du menu en dépend. Les pages d'état et des sources le relisent à
  // chaque ouverture.
  const sources = useSourcesStatus(auth === 'in', false)
  const attention = (sources.status?.sources ?? []).some((source) => needsAction(sourceCard(source, now).state))

  // La sonde lit les cafés d'aujourd'hui dans le cache : la page du jour les a déjà en arrivant.
  // Une panne n'est pas une déconnexion : la page du jour saura la signaler.
  useEffect(() => {
    queryClient.fetchQuery(dayQuery(parisDate(Date.now()))).then(
      () => setAuth('in'),
      (error) => setAuth(error instanceof Unauthorized ? 'out' : 'in'),
    )
  }, [queryClient])

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
  if (route.page === 'authorization') {
    page = <AuthorizationPage menu={menu} statusHref={routePath(STATUS)} onStatus={() => go(STATUS)} onSessionLost={onSessionLost} />
  } else if (route.page === 'status') {
    page = <StatusPage menu={menu} now={now} />
  } else if (route.page === 'sources') {
    page = <SourcesPage menu={menu} now={now} />
  } else if (route.page === 'year') {
    page = (
      <YearPage
        menu={menu}
        year={route.year}
        today={today}
        onYear={(next) => go({ page: 'year', year: next })}
        onOpenMonth={(month) => go({ page: 'month', month })}
        storedMetrics={metricSelection.stored}
        onToggleMetric={metricSelection.toggle}
        onSetAllMetrics={metricSelection.setAll}
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
        onMonth={(month) => go({ page: 'month', month })}
        onOpenDay={openDay}
        storedMetrics={metricSelection.stored}
        onToggleMetric={metricSelection.toggle}
        onSetAllMetrics={metricSelection.setAll}
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
        storedMetrics={metricSelection.stored}
        onToggleMetric={metricSelection.toggle}
        onSetAllMetrics={metricSelection.setAll}
        coffee={coffee}
        music={music}
      />
    )
  }

  return (
    <div className="shell">
      <Sidebar
        wide={wide}
        open={drawerOpen}
        onClose={closeMenu}
        // Le retour de Google fait partie de la page d'état.
        current={
          route.page === 'status' || route.page === 'authorization' ? 'status' : route.page === 'sources' ? 'sources' : 'calendar'
        }
        calendarHref={routePath(TODAY)}
        onCalendar={() => go(TODAY)}
        statusHref={routePath(STATUS)}
        onStatus={() => go(STATUS)}
        sourcesHref={routePath(SOURCES)}
        onSources={() => go(SOURCES)}
        attention={attention}
        onLogout={async () => {
          await logout().catch(() => undefined)
          // L'écran de connexion d'abord : plus aucune lecture ne part, puis le cache est vidé.
          setAuth('out')
          queryClient.clear()
          coffee.reset()
          music.reset()
          setLastDay(null)
          setMenuOpen(false)
          navigate(TODAY, true)
        }}
      />
      {/* `inert` : le tiroir ouvert est seul à recevoir le focus et les touchers. */}
      <div className="content" inert={drawerOpen}>
        {page}
      </div>
    </div>
  )
}
