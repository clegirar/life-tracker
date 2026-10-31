import { useCallback, useEffect, useState } from 'react'
import { fetchDay, logout, Unauthorized } from './api'
import { DayPage } from './DayPage'
import { Login } from './Login'
import { parisDate } from './time'
import { useCoffee } from './useCoffee'

type Auth = 'unknown' | 'in' | 'out' | 'expired'

export function App() {
  const [auth, setAuth] = useState<Auth>('unknown')
  // `null` : suivre aujourd'hui, y compris quand minuit passe avec la page ouverte.
  const [selected, setSelected] = useState<string | null>(null)
  const [now, setNow] = useState(() => Date.now())
  const today = parisDate(now)
  const date = selected ?? today

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
  return (
    <DayPage
      date={date}
      today={today}
      onSelect={(next) => setSelected(next === today ? null : next)}
      coffee={coffee}
      onLogout={async () => {
        await logout().catch(() => undefined)
        coffee.reset()
        setSelected(null)
        setAuth('out')
      }}
    />
  )
}
