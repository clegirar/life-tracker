// `/autorisation` : Google y revient après un accord. Le cookie `SameSite=Strict` n'a pas accompagné
// cette navigation venue d'ailleurs ; la page, elle, appelle le backend avec la session. Elle transmet la
// réponse une seule fois, puis retire le code de l'adresse.

import { useCallback, useEffect, useRef, useState, type MouseEvent, type ReactNode } from 'react'
import { sendAuthorizationCallback, Unauthorized } from './api'
import { callbackFrom, OUTCOME_TEXT, outcomeOf, outcomeTone, storedOutcome, type Outcome } from './authorization'

interface Props {
  /** Le bouton du tiroir, sur iPhone. */
  menu: ReactNode
  statusHref: string
  onStatus: () => void
  onSessionLost: () => void
}

const PATH = '/autorisation'

function Mark({ tone }: { tone: 'ok' | 'todo' | 'broken' }) {
  return (
    <span className="auth-mark" data-state={tone} aria-hidden="true">
      <svg width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.6" strokeLinecap="round" strokeLinejoin="round">
        {tone === 'ok' ? <path d="M5 12.5l4.5 4.5L19 7.5" /> : <path d="M12 7v6M12 17h.01" />}
      </svg>
    </span>
  )
}

export function AuthorizationPage({ menu, statusHref, onStatus, onSessionLost }: Props) {
  // Lue au premier rendu : après l'envoi, l'adresse ne la porte plus.
  const [callback] = useState(() => callbackFrom(window.location.search))
  const [outcome, setOutcome] = useState<Outcome | null>(() =>
    callback ? null : (storedOutcome(window.history.state) ?? 'expired'),
  )
  const sent = useRef(false)

  const deliver = useCallback(async () => {
    if (!callback) return
    try {
      const result = outcomeOf(callback, await sendAuthorizationCallback(callback))
      // Le code quitte l'adresse ; l'issue reste dans l'entrée d'historique, pour un rechargement.
      window.history.replaceState({ authorization: result }, '', PATH)
      setOutcome(result)
    } catch (error) {
      if (error instanceof Unauthorized) {
        // L'adresse garde la réponse : après la connexion, la page la transmet.
        sent.current = false
        onSessionLost()
        return
      }
      setOutcome('failed')
    }
  }, [callback, onSessionLost])

  useEffect(() => {
    if (!callback || sent.current) return
    sent.current = true
    void deliver()
  }, [callback, deliver])

  const retry = () => {
    setOutcome(null)
    void deliver()
  }

  const toStatus = (event: MouseEvent<HTMLAnchorElement>) => {
    if (event.metaKey || event.ctrlKey || event.shiftKey || event.altKey || event.button !== 0) return
    event.preventDefault()
    onStatus()
  }

  const text = outcome ? OUTCOME_TEXT[outcome] : null

  return (
    <main className="page auth-page">
      {menu && <div className="topbar">{menu}</div>}
      <header className="header">
        <div className="header-text">
          <p className="subtitle">Accord Google</p>
          <h1 className="title">Autorisation</h1>
        </div>
      </header>

      <section className="card auth-card" aria-live="polite" data-testid="authorization">
        {text && outcome ? (
          <>
            <Mark tone={outcomeTone(outcome)} />
            <h2 className="auth-title" data-testid="authorization-title">
              {text.title}
            </h2>
            <p className="auth-text">{text.text}</p>
            {outcome === 'failed' ? (
              <button type="button" className="primary auth-action" onClick={retry}>
                Réessayer
              </button>
            ) : (
              <a className="primary auth-action" href={statusHref} onClick={toStatus}>
                {outcome === 'received' ? "Voir l'état" : 'Ouvrir la page État'}
              </a>
            )}
          </>
        ) : (
          <p className="auth-text" role="status">
            Transmission de l'accord…
          </p>
        )}
      </section>
    </main>
  )
}
