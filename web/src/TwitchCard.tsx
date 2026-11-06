import { useRef, useState } from 'react'
import type { MonthDay, SourcesHealth } from './api'
import { MoreButton } from './MusicCard'
import { visibleRows } from './music'
import { channelRows, overlapText, streamMinutesText, twitchEmptyText, twitchSilent, watchedTwitch, type ChannelRow } from './twitch'
import type { useTwitch } from './useTwitch'
import { OutIcon } from './VideoCard'

interface Props {
  date: string
  today: string
  state: ReturnType<typeof useTwitch>
  health: SourcesHealth | null
  /** La journée vue par le mois : sa valeur `null` dit qu'il n'y a pas encore de données Twitch. `null` tant qu'il n'est pas lu. */
  calendarDay: MonthDay | null
}

function ChannelLine({ row }: { row: ChannelRow }) {
  const detail = [...row.contents, row.span].filter(Boolean).join(' · ')
  const text = (
    <div className="row-text">
      <p className={row.name === null ? 'row-name row-name-muted' : 'row-name'}>{row.name ?? 'Chaîne inconnue'}</p>
      {detail && <p className="row-detail">{detail}</p>}
    </div>
  )
  const minutes = <span className="row-count">{row.minutes}</span>
  // Toute la ligne est le lien, comme les vidéos YouTube : une cible de doigt pleine largeur.
  return (
    <li data-testid="twitch-channel">
      {row.url ? (
        <a className="row video-link" href={row.url} target="_blank" rel="noopener noreferrer">
          {text}
          <span className="video-end">
            {minutes}
            <OutIcon />
          </span>
        </a>
      ) : (
        <div className="row">
          {text}
          {minutes}
        </div>
      )}
    </li>
  )
}

export function TwitchCard({ date, today, state, health, calendarDay }: Props) {
  // Déplié pour un jour : changer de jour replie, même en revenant.
  const [unfoldedOn, setUnfoldedOn] = useState<string | null>(null)
  const channelsSection = useRef<HTMLElement>(null)
  const { stream, loadFailed } = state
  const unfolded = unfoldedOn === date

  const watched = stream !== null && watchedTwitch(stream)
  const rows = watched ? channelRows(stream) : []
  const overlap = watched ? overlapText(stream) : null
  const shownRows = visibleRows(rows, unfolded).shown

  return (
    <section className="card card-twitch" aria-labelledby="titre-twitch" data-testid="twitch">
      <h2 id="titre-twitch" className="card-title card-title-twitch">
        Twitch
      </h2>

      {loadFailed && (
        <div className="inline-failure" role="status">
          <span>Impossible d'actualiser les streams.</span>
          <button type="button" className="notice-action" onClick={() => void state.reload()}>
            Réessayer
          </button>
        </div>
      )}
      {twitchSilent(health) && (
        <p className="inline-note" role="status" data-testid="twitch-alert">
          <strong>Firefox ne donne plus signe de vie.</strong> Des streams de ce jour peuvent manquer.
        </p>
      )}

      {!stream && !loadFailed && <p className="empty">Chargement…</p>}
      {stream && !watched && <p className="empty">{twitchEmptyText(date, today, health, calendarDay ? (calendarDay.values['stream.watch'] ?? null) : undefined)}</p>}

      {watched && (
        <>
          <dl className="stats">
            <div>
              <dt>Temps sur Twitch</dt>
              <dd className="stat-value" data-testid="twitch-minutes">
                {streamMinutesText(stream.minutes)}
              </dd>
              {overlap && (
                <dd className="stat-note" data-testid="twitch-overlap">
                  {overlap}
                </dd>
              )}
            </div>
            <div>
              <dt>Chaînes</dt>
              <dd className="stat-value" data-testid="twitch-channel-count">
                {stream.channels.length}
              </dd>
            </div>
          </dl>

          <section ref={channelsSection} className="music-section" aria-labelledby="titre-chaines-twitch" data-testid="twitch-channels">
            <h3 id="titre-chaines-twitch" className="subsection-title">
              Chaînes regardées
            </h3>
            <ul className="rows video-rows">
              {shownRows.map((row) => (
                <ChannelLine key={row.key} row={row} />
              ))}
            </ul>
            <MoreButton total={rows.length} expanded={unfolded} onToggle={() => setUnfoldedOn(unfolded ? null : date)} section={channelsSection} />
          </section>
        </>
      )}
    </section>
  )
}
