import { useRef, useState } from 'react'
import type { MonthDay, SourcesHealth } from './api'
import { MoreButton } from './MusicCard'
import { visibleRows } from './music'
import { discoveryRows, gameRows, playedSteam, playMinutesText, steamEmptyText, steamSilent, type DiscoveryRow, type GameRow } from './steam'
import type { useSteam } from './useSteam'
import { OutIcon } from './VideoCard'
import { CardSkeleton } from './CardSkeleton'

interface Props {
  date: string
  today: string
  state: ReturnType<typeof useSteam>
  health: SourcesHealth | null
  /** La journée vue par le mois : sa valeur `null` dit qu'il n'y a pas encore de données Steam. `null` tant qu'il n'est pas lu. */
  calendarDay: MonthDay | null
}

/** Une coupe, comme le trophée des succès de Steam : petite, dans la couleur de la carte. */
function TrophyIcon() {
  return (
    <svg className="game-trophy" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M8 4h8v5a4 4 0 0 1-8 0zM8 6H5a3 3 0 0 0 3 4M16 6h3a3 3 0 0 1-3 4M12 13v4M8.5 20h7M10 17h4" />
    </svg>
  )
}

function GameLine({ row }: { row: GameRow }) {
  const text = (
    <div className="row-text">
      <p className="row-name">{row.name}</p>
      {row.detail && <p className="row-detail">{row.detail}</p>}
      {row.achievements.length > 0 && (
        <ul className="game-achievements" aria-label={`Succès de ${row.name}`}>
          {row.achievements.map((achievement) => (
            <li key={achievement.key} data-testid="steam-achievement">
              <TrophyIcon />
              <span className="game-achievement-name">{achievement.name}</span>
              <span className="game-achievement-time">{achievement.time}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  )
  const minutes = row.minutes && <span className="row-count">{row.minutes}</span>
  // Toute la ligne ouvre la page du jeu sur le magasin Steam, comme les chaînes Twitch.
  return (
    <li data-testid="steam-game">
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

/** Un jeu découvert : son nom, et toute la ligne ouvre sa page sur le magasin Steam. */
function DiscoveryLine({ row }: { row: DiscoveryRow }) {
  const name = (
    <div className="row-text">
      <p className="row-name">{row.name}</p>
    </div>
  )
  return (
    <li data-testid="steam-discovery">
      {row.url ? (
        <a className="row video-link" href={row.url} target="_blank" rel="noopener noreferrer">
          {name}
          <span className="video-end">
            <OutIcon />
          </span>
        </a>
      ) : (
        <div className="row">{name}</div>
      )}
    </li>
  )
}

export function SteamCard({ date, today, state, health, calendarDay }: Props) {
  // Déplié pour un jour : changer de jour replie, même en revenant.
  const [unfoldedOn, setUnfoldedOn] = useState<string | null>(null)
  const gamesSection = useRef<HTMLElement>(null)
  const { games, loadFailed } = state
  const unfolded = unfoldedOn === date

  const played = games !== null && playedSteam(games)
  const rows = played ? gameRows(games) : []
  const shownRows = visibleRows(rows, unfolded).shown
  const finds = games !== null ? discoveryRows(games) : []

  return (
    <section className="card card-steam" aria-labelledby="titre-steam" data-testid="steam">
      <h2 id="titre-steam" className="card-title card-title-steam">
        Steam
      </h2>

      {loadFailed && (
        <div className="inline-failure" role="status">
          <span>Impossible d'actualiser les parties.</span>
          <button type="button" className="notice-action" onClick={() => void state.reload()}>
            Réessayer
          </button>
        </div>
      )}
      {steamSilent(health) && (
        <p className="inline-note" role="status" data-testid="steam-alert">
          <strong>Le collecteur Steam ne donne plus signe de vie.</strong> Des parties de ce jour peuvent manquer.
        </p>
      )}

      {!games && !loadFailed && <CardSkeleton rows={3} />}
      {games && !played && <p className="empty">{steamEmptyText(date, today, health, calendarDay ? (calendarDay.values['game.play'] ?? null) : undefined)}</p>}

      {played && (
        <>
          <dl className="stats">
            {/* Un succès seul ne dit pas qu'on n'a pas joué (une partie hors ligne se compte plus tard) : pas de zéro. */}
            {games.segments > 0 && (
              <div>
                <dt>Temps de jeu</dt>
                <dd className="stat-value" data-testid="steam-minutes">
                  {playMinutesText(games.minutes)}
                </dd>
              </div>
            )}
            {games.achievements > 0 && (
              <div>
                <dt>Succès</dt>
                <dd className="stat-value" data-testid="steam-achievement-count">
                  {games.achievements}
                </dd>
              </div>
            )}
          </dl>

          {/* Un jour sans nouveau jeu est la règle : pas de section du tout, plutôt qu'un « aucune ». */}
          {finds.length > 0 && (
            <section className="music-section" aria-labelledby="titre-decouvertes-steam" data-testid="steam-discoveries">
              <h3 id="titre-decouvertes-steam" className="subsection-title">
                Découvertes
                <span className="subsection-count subsection-count-steam">{finds.length}</span>
              </h3>
              <ul className="rows video-rows">
                {finds.map((row) => (
                  <DiscoveryLine key={row.key} row={row} />
                ))}
              </ul>
            </section>
          )}

          <section ref={gamesSection} className="music-section" aria-labelledby="titre-jeux-steam" data-testid="steam-games">
            <h3 id="titre-jeux-steam" className="subsection-title">
              Jeux
            </h3>
            <ul className="rows video-rows">
              {shownRows.map((row) => (
                <GameLine key={row.key} row={row} />
              ))}
            </ul>
            <MoreButton total={rows.length} expanded={unfolded} onToggle={() => setUnfoldedOn(unfolded ? null : date)} section={gamesSection} />
          </section>
        </>
      )}
    </section>
  )
}
