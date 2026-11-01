import { useState } from 'react'
import { discoveryDetail, markFailureText } from './discoveries'
import { formatMinutes, musicEmptyText, plural } from './music'
import type { useMusic } from './useMusic'

type Tab = 'tracks' | 'artists' | 'albums'

const TABS: [Tab, string][] = [
  ['tracks', 'Titres'],
  ['artists', 'Artistes'],
  ['albums', 'Albums'],
]

interface Props {
  date: string
  today: string
  state: ReturnType<typeof useMusic>
}

interface Row {
  name: string
  detail: string
  listens: number
}

export function MusicCard({ date, today, state }: Props) {
  const [tab, setTab] = useState<Tab>('tracks')
  const { music, health, loadFailed, firstListens } = state

  const rows: Row[] = !music
    ? []
    : tab === 'artists'
      ? music.artists.map((a) => ({ name: a.name, detail: plural(a.tracks, 'titre'), listens: a.listens }))
      : (tab === 'tracks' ? music.tracks : music.albums).map((r) => ({ name: r.name, detail: r.artist ?? 'Artiste inconnu', listens: r.listens }))

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

      {!music && !loadFailed && <p className="empty">Chargement…</p>}
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

          <section aria-labelledby="titre-decouvertes" data-testid="discoveries">
            <h3 id="titre-decouvertes" className="subsection-title">
              Découvertes
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
            {firstListens.length === 0 ? (
              <p className="empty">Aucune découverte ce jour-là.</p>
            ) : (
              <ul className="rows finds">
                {firstListens.map((row) => (
                  <li key={`${row.level}/${row.item_key}`} className="find">
                    <div className="row-text">
                      <p className="row-name">{row.name}</p>
                      <p className="row-detail">{discoveryDetail(row, row.known)}</p>
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
                ))}
              </ul>
            )}
          </section>

          <div className="segments" role="group" aria-label="Liste du jour">
            {TABS.map(([id, label]) => (
              <button key={id} type="button" className="segment" aria-pressed={tab === id} onClick={() => setTab(id)}>
                {label}
              </button>
            ))}
          </div>

          {rows.length === 0 ? (
            <p className="empty">
              {tab === 'albums' ? 'Aucun album indiqué par la source.' : 'Aucun artiste connu pour ces écoutes.'}
            </p>
          ) : (
            <ul className="rows">
              {rows.map((row) => (
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
        </>
      )}
    </section>
  )
}
