import { useRef, useState, type ReactNode, type RefObject } from 'react'
import type { MonthDay, SourcesHealth } from './api'
import { MoreButton } from './MusicCard'
import { visibleRows } from './music'
import {
  gameSections,
  gamesEmptyText,
  MATCH_LIMIT,
  playedGames,
  playMinutesText,
  sectionSummary,
  silenceText,
  silentGameServices,
  type GameRow,
  type GameSectionId,
  type MatchRow,
} from './games'
import { useFoldedSections } from './useFoldedSections'
import type { useGames } from './useGames'
import { OutIcon } from './VideoCard'
import { CardSkeleton } from './CardSkeleton'

interface Props {
  date: string
  today: string
  state: ReturnType<typeof useGames>
  health: SourcesHealth | null
  /** La journée vue par le mois : sa valeur `null` dit qu'il n'y a pas encore de données de jeu. `null` tant qu'il n'est pas lu. */
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

/**
 * Une partie, sous son jeu, comme une ligne d'historique de match : l'heure, le résultat en couleur,
 * ce que j'ai joué avec la file dessous, la durée.
 */
function MatchLine({ match }: { match: MatchRow }) {
  return (
    <li className="game-match" data-testid="game-match">
      <span className="game-match-time">{match.time}</span>
      {match.result && <span className={`game-match-result game-match-${match.tone}`}>{match.result}</span>}
      <span className="game-match-text">
        <span className="game-match-title">{match.title}</span>
        {match.detail && <span className="game-match-detail">{match.detail}</span>}
      </span>
      <span className="game-match-minutes">{match.minutes}</span>
    </li>
  )
}

/** Les parties d'un jeu : les cinq premières, puis « N autres parties » pour un soir chargé. */
function MatchList({ row, unfolded, onUnfold }: { row: GameRow; unfolded: boolean; onUnfold: () => void }) {
  // Replier pour ne cacher qu'une partie n'aurait pas de sens : la limite ne joue qu'à partir de deux.
  const folded = !unfolded && row.matches.length > MATCH_LIMIT + 1
  const shown = folded ? row.matches.slice(0, MATCH_LIMIT) : row.matches
  return (
    <>
      <ul className="game-matches" aria-label={`Parties de ${row.name}`}>
        {shown.map((match) => (
          <MatchLine key={match.key} match={match} />
        ))}
      </ul>
      {folded && (
        <button type="button" className="game-matches-more" onClick={onUnfold}>
          {row.matches.length - MATCH_LIMIT} autres parties
        </button>
      )}
    </>
  )
}

function GameLine({ row, matchesUnfolded, onUnfoldMatches }: { row: GameRow; matchesUnfolded: boolean; onUnfoldMatches: () => void }) {
  const text = (
    <div className="row-text">
      <p className="row-name">
        {row.name}
        {/* Joué ce jour-là pour la première fois : une étiquette sur sa ligne, plutôt qu'une liste à part qui le répéterait. */}
        {row.discovery && (
          <span className="game-discovery" data-testid="game-discovery">
            Découverte
          </span>
        )}
      </p>
      {row.detail && <p className="row-detail">{row.detail}</p>}
      {row.achievements.length > 0 && (
        <ul className="game-achievements" aria-label={`Succès de ${row.name}`}>
          {row.achievements.map((achievement) => (
            <li key={achievement.key} data-testid="game-achievement">
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
    <li data-testid="game">
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
      {/* Les parties sous la ligne du jeu, sur toute la largeur : leurs durées s'alignent sur la sienne. */}
      {row.matches.length > 0 && <MatchList row={row} unfolded={matchesUnfolded} onUnfold={onUnfoldMatches} />}
    </li>
  )
}

/** Le chevron de la divulgation : vers le bas ouverte, vers la droite pliée. */
function Chevron() {
  return (
    <svg className="games-section-chevron" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.6" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M6 9l6 6 6-6" />
    </svg>
  )
}

/**
 * Une section pliable : l'intertitre est le bouton. Pliée, il garde un résumé (« 5 h 49 · 12 parties »).
 * Le contenu reste dans la page, rendu inerte : il se replie en hauteur par une transition de
 * `grid-template-rows`, interruptible, sans mesurer.
 */
function GameSection({
  id,
  title,
  rows,
  folded,
  onToggle,
  sectionRef,
  children,
}: {
  id: GameSectionId
  title: string
  rows: GameRow[]
  folded: boolean
  onToggle: () => void
  sectionRef?: RefObject<HTMLElement | null>
  children: ReactNode
}) {
  const body = `jeux-${id}-contenu`
  const summary = sectionSummary(rows, id)
  return (
    <section ref={sectionRef} className="music-section games-section" aria-labelledby={`titre-jeux-${id}`} data-testid={`games-${id}`} data-folded={folded}>
      <h3 className="subsection-title games-section-title">
        <button type="button" id={`titre-jeux-${id}`} className="games-section-toggle" aria-expanded={!folded} aria-controls={body} onClick={onToggle}>
          <Chevron />
          <span className="games-section-name">{title}</span>
          {folded && summary && (
            <span className="games-section-summary" data-testid="games-section-summary">
              {summary}
            </span>
          )}
        </button>
      </h3>
      <div id={body} className="games-section-body" inert={folded}>
        <div className="games-section-inner">{children}</div>
      </div>
    </section>
  )
}

/**
 * La carte « Jeux » : le temps du jour, les parties et les succès, puis deux sections, « Steam » et
 * « Riot ». Une section sans rien ce jour-là ne s'affiche pas (ni titre ni « aucun ») : la journée vide
 * se dit pour toute la carte, et le silence d'une source par l'alerte du haut.
 */
export function GamesCard({ date, today, state, health, calendarDay }: Props) {
  // Déplié pour un jour : changer de jour replie, même en revenant.
  const [unfoldedOn, setUnfoldedOn] = useState<string | null>(null)
  // Les jeux dont les parties sont dépliées, pour un jour : changer de jour replie.
  const [matchesOpen, setMatchesOpen] = useState<{ date: string; games: string[] }>({ date: '', games: [] })
  const steamSection = useRef<HTMLElement>(null)
  const { folded, toggle } = useFoldedSections()
  const { games, loadFailed } = state
  const unfolded = unfoldedOn === date
  const openGames = matchesOpen.date === date ? matchesOpen.games : []

  const played = games !== null && playedGames(games)
  const sections = played ? gameSections(games) : { steam: [], riot: [] }
  const shownSteam = visibleRows(sections.steam, unfolded).shown
  const silent = silentGameServices(health)

  const line = (row: GameRow) => (
    <GameLine
      key={row.key}
      row={row}
      matchesUnfolded={openGames.includes(row.key)}
      onUnfoldMatches={() => setMatchesOpen({ date, games: [...openGames, row.key] })}
    />
  )

  return (
    <section className="card card-games" aria-labelledby="titre-jeux" data-testid="games">
      <h2 id="titre-jeux" className="card-title card-title-games">
        Jeux
      </h2>

      {loadFailed && (
        <div className="inline-failure" role="status">
          <span>Impossible d'actualiser les parties.</span>
          <button type="button" className="notice-action" onClick={() => void state.reload()}>
            Réessayer
          </button>
        </div>
      )}
      {silent.length > 0 && (
        <p className="inline-note" role="status" data-testid="games-alert">
          <strong>{silenceText(silent)}</strong> Des parties de ce jour peuvent manquer.
        </p>
      )}

      {!games && !loadFailed && <CardSkeleton rows={3} />}
      {games && !played && <p className="empty">{gamesEmptyText(date, today, health, calendarDay ? (calendarDay.values['game.play'] ?? null) : undefined)}</p>}

      {played && (
        <>
          <dl className="stats">
            {/* Un succès seul ne dit pas qu'on n'a pas joué (une partie hors ligne se compte plus tard) : pas de zéro. */}
            {games.segments > 0 && (
              <div>
                <dt>Temps de jeu</dt>
                <dd className="stat-value" data-testid="games-minutes">
                  {playMinutesText(games.minutes)}
                </dd>
              </div>
            )}
            {games.matches.length > 0 && (
              <div>
                <dt>Parties</dt>
                <dd className="stat-value" data-testid="games-match-count">
                  {games.matches.length}
                </dd>
              </div>
            )}
            {games.achievements > 0 && (
              <div>
                <dt>Succès</dt>
                <dd className="stat-value" data-testid="games-achievement-count">
                  {games.achievements}
                </dd>
              </div>
            )}
          </dl>

          {/* Pliées ou non, pour tous les jours : un réglage du lecteur, gardé dans le navigateur. */}
          {sections.steam.length > 0 && (
            <GameSection id="steam" title="Steam" rows={sections.steam} folded={folded.includes('steam')} onToggle={() => toggle('steam')} sectionRef={steamSection}>
              <ul className="rows video-rows">{shownSteam.map(line)}</ul>
              <MoreButton total={sections.steam.length} expanded={unfolded} onToggle={() => setUnfoldedOn(unfolded ? null : date)} section={steamSection} />
            </GameSection>
          )}

          {sections.riot.length > 0 && (
            <GameSection id="riot" title="Riot" rows={sections.riot} folded={folded.includes('riot')} onToggle={() => toggle('riot')}>
              <ul className="rows video-rows">{sections.riot.map(line)}</ul>
            </GameSection>
          )}
        </>
      )}
    </section>
  )
}
