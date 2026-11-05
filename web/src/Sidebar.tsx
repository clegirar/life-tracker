// La barre latérale : fixe à gauche sur un écran large, tiroir derrière un bouton sur iPhone.
// Deux rubriques : le calendrier et l'état des trackers, marqué d'un point quand une carte attend une action.

import { useEffect, useRef, useSyncExternalStore, type MouseEvent, type RefObject } from 'react'

/** Le point de rupture de la barre fixe, le même que dans style.css. */
const WIDE = '(min-width: 900px)'

function subscribe(onChange: () => void) {
  const query = window.matchMedia(WIDE)
  query.addEventListener('change', onChange)
  return () => query.removeEventListener('change', onChange)
}

export const useWide = () => useSyncExternalStore(subscribe, () => window.matchMedia(WIDE).matches)

function CalendarIcon() {
  return (
    <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <rect x="3.5" y="5" width="17" height="15.5" rx="3" />
      <path d="M3.5 10h17M8 3v4M16 3v4" />
    </svg>
  )
}

function StatusIcon() {
  return (
    <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M3 12h4l2.5-6 5 12 2.5-6h4" />
    </svg>
  )
}

interface MenuButtonProps {
  open: boolean
  onOpen: () => void
  buttonRef: RefObject<HTMLButtonElement | null>
  /** Une carte de la page d'état attend une action : le tiroir fermé le dit aussi. */
  attention: boolean
}

export function MenuButton({ open, onOpen, buttonRef, attention }: MenuButtonProps) {
  return (
    <button
      ref={buttonRef}
      type="button"
      className="round menu-button"
      aria-label="Menu"
      aria-expanded={open}
      aria-controls="barre-laterale"
      onClick={onOpen}
    >
      <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" aria-hidden="true">
        <path d="M4 7h16M4 12h16M4 17h16" />
      </svg>
      {attention && <span className="menu-dot" data-testid="menu-dot" aria-hidden="true" />}
    </button>
  )
}

interface Props {
  wide: boolean
  open: boolean
  onClose: () => void
  /** La rubrique ouverte. */
  current: 'calendar' | 'status'
  /** L'adresse de l'entrée « Calendrier ». */
  calendarHref: string
  onCalendar: () => void
  statusHref: string
  onStatus: () => void
  /** Une carte de la page d'état attend une action. */
  attention: boolean
  onLogout: () => void
}

export function Sidebar({ wide, open, onClose, current, calendarHref, onCalendar, statusHref, onStatus, attention, onLogout }: Props) {
  const first = useRef<HTMLAnchorElement>(null)
  const drawer = !wide
  const shown = wide || open

  // Le focus entre dans le tiroir à l'ouverture ; Échap le ferme.
  useEffect(() => {
    if (!drawer || !open) return
    first.current?.focus({ preventScroll: true })
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose()
    }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [drawer, open, onClose])

  const go = (action: () => void) => (event: MouseEvent<HTMLAnchorElement>) => {
    // Un clic avec modificateur garde son sens : nouvel onglet, nouvelle fenêtre.
    if (event.metaKey || event.ctrlKey || event.shiftKey || event.altKey || event.button !== 0) return
    event.preventDefault()
    action()
  }

  return (
    <>
      {drawer && <div className="scrim" data-open={open} aria-hidden="true" onClick={onClose} />}
      <aside
        id="barre-laterale"
        className="sidebar"
        data-open={open}
        role={drawer ? 'dialog' : undefined}
        aria-modal={drawer ? true : undefined}
        aria-label="Menu"
        inert={!shown}
      >
        <p className="sidebar-title">Life tracker</p>
        <nav aria-label="Rubriques">
          <ul className="sidebar-list">
            <li>
              <a
                ref={first}
                href={calendarHref}
                className="sidebar-item"
                aria-current={current === 'calendar' ? 'page' : undefined}
                onClick={go(onCalendar)}
              >
                <CalendarIcon />
                Calendrier
              </a>
            </li>
            <li>
              <a
                href={statusHref}
                className="sidebar-item"
                // Le point ne se lit pas : le nom le dit.
                aria-label={attention ? 'État, une action attendue' : undefined}
                aria-current={current === 'status' ? 'page' : undefined}
                onClick={go(onStatus)}
              >
                <StatusIcon />
                État
                {attention && <span className="sidebar-dot" data-testid="status-dot" aria-hidden="true" />}
              </a>
            </li>
          </ul>
        </nav>
        <button type="button" className="sidebar-item sidebar-logout" onClick={onLogout}>
          Se déconnecter
        </button>
      </aside>
    </>
  )
}
