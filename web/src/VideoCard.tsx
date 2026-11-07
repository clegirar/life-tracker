import { useRef, useState } from 'react'
import type { SourcesHealth } from './api'
import { MoreButton } from './MusicCard'
import { plural, visibleRows } from './music'
import type { useVideo } from './useVideo'
import { channelOfDay, totalText, videoEmptyText, watchRows, youtubeSilent, type WatchRow } from './video'

interface Props {
  date: string
  today: string
  state: ReturnType<typeof useVideo>
  health: SourcesHealth | null
}

/** La flèche des liens qui sortent de l'app. */
export function OutIcon() {
  return (
    <svg className="video-out" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.4" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M8 16L16 8M9 8h7v7" />
    </svg>
  )
}

/**
 * La miniature, décorative : le titre fait déjà le lien. Son cadre 16/9 est posé avant l'image, rien ne
 * bouge à son arrivée ; si Google ne la rend pas, le cadre reste, vide, et la liste garde son alignement.
 */
function Thumbnail({ src }: { src: string }) {
  const [failed, setFailed] = useState<string | null>(null)
  return (
    <span className="video-thumb" data-testid="video-thumb">
      {failed !== src && (
        <img src={src} alt="" width={320} height={180} loading="lazy" decoding="async" referrerPolicy="no-referrer" onError={() => setFailed(src)} />
      )}
    </span>
  )
}

function WatchLine({ row }: { row: WatchRow }) {
  const detail = [row.time, row.channel, row.seenBefore ? 'déjà vue' : null].filter(Boolean).join(' · ')
  const text = (
    <div className="row-text">
      <p className={row.title === null ? 'row-name row-name-muted' : 'row-name'}>{row.title ?? 'Vidéo indisponible'}</p>
      <p className="row-detail">{detail}</p>
    </div>
  )
  const length = row.length && <span className="row-count">{row.length}</span>
  // Toute la ligne est le lien : une cible de doigt, comme une cellule d'iPhone.
  return (
    <li data-testid="video-watch">
      {row.url ? (
        <a className="row video-link" href={row.url} target="_blank" rel="noopener noreferrer">
          {row.thumbnail && <Thumbnail src={row.thumbnail} />}
          {text}
          <span className="video-end">
            {length}
            <OutIcon />
          </span>
        </a>
      ) : (
        <div className="row">
          {text}
          {length}
        </div>
      )}
    </li>
  )
}

export function VideoCard({ date, today, state, health }: Props) {
  // Déplié pour un jour : changer de jour replie, même en revenant.
  const [unfolded, setUnfolded] = useState({ date: '', watches: false })
  const watchesSection = useRef<HTMLElement>(null)
  const { video, loadFailed } = state
  const open = unfolded.date === date ? unfolded : { date, watches: false }
  const toggle = () => setUnfolded({ ...open, watches: !open.watches })

  const watched = video !== null && video.count > 0
  const rows = watched ? watchRows(video) : []
  const topChannel = watched ? channelOfDay(video.watches) : null
  const shownRows = visibleRows(rows, open.watches).shown

  return (
    <section className="card card-video" aria-labelledby="titre-youtube" data-testid="video">
      <h2 id="titre-youtube" className="card-title card-title-video">
        YouTube
      </h2>

      {loadFailed && (
        <div className="inline-failure" role="status">
          <span>Impossible d'actualiser les vidéos.</span>
          <button type="button" className="notice-action" onClick={() => void state.reload()}>
            Réessayer
          </button>
        </div>
      )}
      {youtubeSilent(health) && (
        <p className="inline-note" role="status" data-testid="youtube-alert">
          <strong>YouTube ne répond plus.</strong> Des vidéos de ce jour peuvent manquer.
        </p>
      )}

      {!video && !loadFailed && <p className="empty">Chargement…</p>}
      {video?.count === 0 && <p className="empty">{videoEmptyText(date, today, health)}</p>}

      {watched && (
        <>
          <dl className="stats">
            <div>
              <dt>Vidéos</dt>
              <dd className="stat-value" data-testid="video-count">
                {video.count}
              </dd>
            </div>
            <div>
              <dt>Durée des vidéos</dt>
              <dd className="stat-value" data-testid="video-duration">
                {totalText(video.seconds)}
              </dd>
              <dd className="stat-note">Pas le temps regardé.</dd>
              {video.without_duration > 0 && (
                <dd className="stat-note" data-testid="video-without-duration">
                  {video.seconds === null
                    ? `Durée inconnue pour ${plural(video.without_duration, 'vidéo')}.`
                    : `Sans compter ${plural(video.without_duration, 'vidéo')} de durée inconnue.`}
                </dd>
              )}
            </div>
            {topChannel && (
              <div className="stat-wide">
                <dt>Chaîne du jour</dt>
                <dd className="stat-artist">
                  {topChannel.name} · {plural(topChannel.count, 'vidéo')}
                </dd>
              </div>
            )}
          </dl>

          <section ref={watchesSection} className="music-section" aria-labelledby="titre-vues" data-testid="video-watches">
            <h3 id="titre-vues" className="subsection-title">
              Vidéos vues
            </h3>
            <ul className="rows video-rows">
              {shownRows.map((row) => (
                <WatchLine key={row.key} row={row} />
              ))}
            </ul>
            <MoreButton total={rows.length} expanded={open.watches} onToggle={toggle} section={watchesSection} />
          </section>
        </>
      )}
    </section>
  )
}
