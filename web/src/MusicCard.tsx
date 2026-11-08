import { useEffect, useRef, useState, type RefObject } from 'react'
import { discoveryDetail, itemId, markFailureText } from './discoveries'
import {
  discoveriesOf,
  formatMinutes,
  LIST_LIMIT,
  musicEmptyText,
  noDiscoveryText,
  plural,
  visibleRows,
  type MusicTab,
} from './music'
import { Segments } from './Segments'
import type { SourcesHealth } from './api'
import type { useMusic } from './useMusic'
import { CardSkeleton } from './CardSkeleton'

const TABS = [
  ['tracks', 'Titres'],
  ['artists', 'Artistes'],
  ['albums', 'Albums'],
] as const

interface Props {
  date: string
  today: string
  state: ReturnType<typeof useMusic>
  health: SourcesHealth | null
}

interface Row {
  name: string
  detail: string
  listens: number
}

/** « Tout afficher (N) », puis « Réduire ». Replier ramène le haut de la section à l'écran s'il en est sorti. */
export function MoreButton({ total, expanded, onToggle, section }: { total: number; expanded: boolean; onToggle: () => void; section: RefObject<HTMLElement | null> }) {
  if (total <= LIST_LIMIT) return null
  return (
    <button
      type="button"
      className="more"
      aria-expanded={expanded}
      onClick={() => {
        onToggle()
        if (!expanded) return
        requestAnimationFrame(() => {
          const el = section.current
          if (el && el.getBoundingClientRect().top < 0) el.scrollIntoView({ block: 'start' })
        })
      }}
    >
      {expanded ? 'Réduire' : `Tout afficher (${total})`}
    </button>
  )
}

export function MusicCard({ date, today, state, health }: Props) {
  const [tab, setTab] = useState<MusicTab>('tracks')
  // Déplié pour un jour et un onglet : changer de jour ou d'onglet replie, même en revenant.
  const [unfolded, setUnfolded] = useState({ key: '', finds: false, rows: false })
  const findsSection = useRef<HTMLElement>(null)
  const rowsSection = useRef<HTMLElement>(null)
  const { music, loadFailed, firstListens } = state
  const key = `${date}/${tab}`
  const open = unfolded.key === key ? unfolded : { key, finds: false, rows: false }
  const toggle = (part: 'finds' | 'rows') => setUnfolded({ ...open, [part]: !open[part] })

  const rows: Row[] = !music
    ? []
    : tab === 'artists'
      ? music.artists.map((a) => ({ name: a.name, detail: plural(a.tracks, 'titre'), listens: a.listens }))
      : (tab === 'tracks' ? music.tracks : music.albums).map((r) => ({ name: r.name, detail: r.artist ?? 'Artiste inconnu', listens: r.listens }))
  const finds = discoveriesOf(firstListens, tab)
  const discovered = finds.filter((row) => !row.known).length
  const shownFinds = visibleRows(finds, open.finds).shown
  const shownRows = visibleRows(rows, open.rows).shown
  // « Déjà connu » ne s'affiche qu'à la demande : au survol sur Mac, au toucher de sa ligne sur iPhone.
  // Une seule ligne révélée à la fois ; toucher ailleurs la referme.
  const [revealed, setRevealed] = useState<string | null>(null)
  useEffect(() => {
    if (revealed === null) return
    const close = (event: PointerEvent) => {
      const row = event.target instanceof Element ? event.target.closest<HTMLElement>('.find') : null
      if (row?.dataset.item !== revealed) setRevealed(null)
    }
    document.addEventListener('pointerdown', close)
    return () => document.removeEventListener('pointerdown', close)
  }, [revealed])

  return (
    <section className="card card-music" aria-labelledby="titre-musique" data-testid="music">
      <h2 id="titre-musique" className="card-title card-title-music">
        Musique
      </h2>

      {loadFailed && (
        <div className="inline-failure" role="status">
          <span>Impossible d'actualiser la musique.</span>
          <button type="button" className="notice-action" onClick={() => void state.reload()}>
            Réessayer
          </button>
        </div>
      )}

      {!music && !loadFailed && <CardSkeleton rows={6} />}
      {music?.listens === 0 && <p className="empty">{musicEmptyText(date, today, health)}</p>}

      {music && music.listens > 0 && (
        <>
          <dl className="stats">
            <div>
              <dt>Écoutes</dt>
              <dd className="stat-value" data-testid="listens">
                {music.listens}
              </dd>
              {music.listens_without_artist > 0 && (
                <dd className="stat-note" data-testid="without-artist">
                  dont {music.listens_without_artist} sans artiste connu.
                </dd>
              )}
            </div>
            <div>
              <dt>Temps estimé</dt>
              <dd className="stat-value" data-testid="minutes">
                {formatMinutes(music.minutes)}
              </dd>
              {music.listens_without_duration > 0 && (
                <dd className="stat-note">
                  {music.minutes === null ? 'Durée inconnue pour ' : 'Sans compter '}
                  {plural(music.listens_without_duration, 'écoute')}
                  {music.minutes === null ? '.' : ' de durée inconnue.'}
                </dd>
              )}
            </div>
            {music.artist_of_day && (
              <div className="stat-wide">
                <dt>Artiste du jour</dt>
                <dd className="stat-artist">
                  {music.artist_of_day.name} · {plural(music.artist_of_day.listens, 'écoute')}
                </dd>
              </div>
            )}
          </dl>

          <Segments
            label="Trier par"
            options={TABS}
            value={tab}
            onChange={(next) => {
              setTab(next)
              setUnfolded({ key: '', finds: false, rows: false })
            }}
            className="music-tabs"
          />

          <section ref={findsSection} className="music-section" aria-labelledby="titre-decouvertes" data-testid="discoveries">
            <h3 id="titre-decouvertes" className="subsection-title">
              Découvertes
              {discovered > 0 && <span className="subsection-count" data-testid="discoveries-count">{discovered}</span>}
            </h3>
            {state.failedMarks.map((op) => (
              <div className="inline-failure" role="alert" key={op.id}>
                <span>{markFailureText(op)}</span>
                <span className="notice-actions">
                  {!op.final && (
                    <button type="button" className="notice-action" onClick={() => state.retryMark(op)}>
                      Réessayer
                    </button>
                  )}
                  <button type="button" className="notice-action" onClick={() => state.dismissMark(op)}>
                    Ignorer
                  </button>
                </span>
              </div>
            ))}
            {state.marksSlow && (
              <p className="empty" role="status">
                C'est bien enregistré. La liste sera confirmée dès que le serveur l'aura traité.
              </p>
            )}
            {finds.length === 0 ? (
              <p className="empty">{noDiscoveryText(tab)}</p>
            ) : (
              <ul className="rows finds">
                {shownFinds.map((row) => {
                  const detail = discoveryDetail(row)
                  const id = itemId(row)
                  const failed = state.failedMarks.some((op) => op.id === id && op.date === date)
                  return (
                    // Le toucher de la ligne révèle son bouton ; le bouton reste la seule commande.
                    <li
                      key={id}
                      className="find"
                      data-item={id}
                      data-revealed={revealed === id ? '' : undefined}
                      data-failed={failed ? '' : undefined}
                      onClick={() => setRevealed(id)}
                    >
                      <div className="row-text">
                        <p className="row-name">{row.name}</p>
                        {detail && <p className="row-detail">{detail}</p>}
                      </div>
                      {/* Pas `disabled` pendant l'envoi : le bouton garderait mal le focus. */}
                      <button
                        type="button"
                        className="known"
                        aria-pressed={row.known}
                        aria-busy={row.pending}
                        aria-label={`${row.name} : déjà connu`}
                        onClick={() => state.toggleKnown(row)}
                      >
                        <span className="known-pill">{row.known ? 'Marqué connu' : 'Déjà connu'}</span>
                      </button>
                    </li>
                  )
                })}
              </ul>
            )}
            <MoreButton total={finds.length} expanded={open.finds} onToggle={() => toggle('finds')} section={findsSection} />
          </section>

          <section ref={rowsSection} className="music-section" aria-labelledby="titre-ecoutes" data-testid="listens-list">
            <h3 id="titre-ecoutes" className="subsection-title">
              Écoutes
            </h3>
            {rows.length === 0 ? (
              <p className="empty">
                {tab === 'albums' ? 'Aucun album indiqué par la source.' : 'Aucun artiste connu pour ces écoutes.'}
              </p>
            ) : (
              <ul className="rows">
                {shownRows.map((row) => (
                  <li key={`${row.name}/${row.detail}`} className="row">
                    <div className="row-text">
                      <p className="row-name">{row.name}</p>
                      <p className="row-detail">{row.detail}</p>
                    </div>
                    <span className="row-count" aria-label={plural(row.listens, 'écoute')}>
                      {row.listens}
                    </span>
                  </li>
                ))}
              </ul>
            )}
            <MoreButton total={rows.length} expanded={open.rows} onToggle={() => toggle('rows')} section={rowsSection} />
          </section>
        </>
      )}
    </section>
  )
}
