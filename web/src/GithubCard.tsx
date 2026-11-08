import { useRef, useState } from 'react'
import type { GithubRepo, MonthDay, SourcesHealth } from './api'
import {
  activeGithub,
  activitySummary,
  addedText,
  coveredValue,
  dayLines,
  deletedText,
  discoveryRows,
  githubEmptyText,
  githubSilent,
  linesLabel,
  repoRows,
  repoSummary,
  repoUrl,
  unknownLinesText,
  type ActivityKind,
  type ActivityRow,
  type DiscoveryRow,
  type Lines,
} from './github'
import { MoreButton } from './MusicCard'
import { visibleRows } from './music'
import type { useGithub } from './useGithub'
import { OutIcon } from './VideoCard'

interface Props {
  date: string
  today: string
  state: ReturnType<typeof useGithub>
  health: SourcesHealth | null
  /** La journée vue par le mois : ses valeurs `null` disent qu'il n'y a pas encore de données GitHub. `null` tant qu'il n'est pas lu. */
  calendarDay: MonthDay | null
}

// Les pictogrammes de GitHub (Octicons), redessinés au trait : on reconnaît l'objet avant de lire la ligne.
const ICON_PATHS: Record<ActivityKind, string> = {
  commit: 'M2 12h6.5M15.5 12H22M12 8.5a3.5 3.5 0 1 0 0 7 3.5 3.5 0 0 0 0-7z',
  pull_request: 'M6 8.5v7M6 3.5a2.5 2.5 0 1 0 0 5 2.5 2.5 0 0 0 0-5zM6 15.5a2.5 2.5 0 1 0 0 5 2.5 2.5 0 0 0 0-5zM18 15.5a2.5 2.5 0 1 0 0 5 2.5 2.5 0 0 0 0-5zM18 15.5V9a3 3 0 0 0-3-3h-3.5M13.5 3.5 11 6l2.5 2.5',
  merged: 'M6 8.5v7M6 3.5a2.5 2.5 0 1 0 0 5 2.5 2.5 0 0 0 0-5zM6 15.5a2.5 2.5 0 1 0 0 5 2.5 2.5 0 0 0 0-5zM18 13.5a2.5 2.5 0 1 0 0-5 2.5 2.5 0 0 0 0 5zM6 8.5c0 2.5 2.5 2.5 9.5 2.5',
  review: 'M2 12s3.6-6.5 10-6.5S22 12 22 12s-3.6 6.5-10 6.5S2 12 2 12zM12 9.5a2.5 2.5 0 1 0 0 5 2.5 2.5 0 0 0 0-5z',
  issue: 'M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM12 11a1 1 0 1 0 0 2 1 1 0 0 0 0-2z',
  comment: 'M20.5 11.5a8 8 0 0 1-11.8 7L3.5 20l1.4-4.9A8 8 0 1 1 20.5 11.5z',
}

function KindIcon({ kind }: { kind: ActivityKind }) {
  return (
    <svg className="gh-icon" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d={ICON_PATHS[kind]} />
    </svg>
  )
}

/** « +40 −2 », en vert et en rouge comme sur GitHub ; « lignes inconnues », jamais un zéro inventé. */
function LinesText({ lines }: { lines: Lines }) {
  if (lines === null) return null
  if (lines === 'unknown') return <span className="gh-lines gh-lines-unknown">lignes inconnues</span>
  return (
    <span className="gh-lines">
      <span aria-hidden="true" data-testid="github-row-lines">
        <span className="gh-add">{addedText(lines.additions)}</span> <span className="gh-del">{deletedText(lines.deletions)}</span>
      </span>
      <span className="visually-hidden">{linesLabel(lines.additions, lines.deletions)}</span>
    </span>
  )
}

function ActivityLine({ row }: { row: ActivityRow }) {
  const text = (
    <>
      <KindIcon kind={row.kind} />
      <div className="row-text">
        <p className={row.untitled ? 'row-name row-name-muted' : 'row-name'}>{row.name}</p>
        <p className="row-detail">{row.detail}</p>
      </div>
    </>
  )
  // Toute la ligne ouvre l'objet sur github.com, comme les jeux sur le magasin Steam.
  return (
    <li data-testid="github-row" data-kind={row.kind}>
      {row.url ? (
        <a className="row video-link gh-row" href={row.url} target="_blank" rel="noopener noreferrer">
          {text}
          <span className="video-end">
            <LinesText lines={row.lines} />
            <OutIcon />
          </span>
        </a>
      ) : (
        <div className="row gh-row">
          {text}
          <LinesText lines={row.lines} />
        </div>
      )}
    </li>
  )
}

function RepoSection({ repo, unfolded, onToggle }: { repo: GithubRepo; unfolded: boolean; onToggle: () => void }) {
  const section = useRef<HTMLElement>(null)
  const rows = repoRows(repo)
  const shown = visibleRows(rows, unfolded).shown
  const url = repoUrl(repo.name)
  const titleId = `titre-depot-${repo.key.replace(/[^A-Za-z0-9_-]/g, '')}`
  return (
    <section ref={section} className="music-section gh-repo" aria-labelledby={titleId} data-testid="github-repo">
      <h3 id={titleId} className="subsection-title gh-repo-title">
        {url ? (
          <a className="gh-repo-link" href={url} target="_blank" rel="noopener noreferrer">
            {repo.name}
          </a>
        ) : (
          repo.name
        )}
      </h3>
      <p className="gh-repo-summary">{repoSummary(repo)}</p>
      <ul className="rows video-rows">
        {shown.map((row) => (
          <ActivityLine key={row.key} row={row} />
        ))}
      </ul>
      <MoreButton total={rows.length} expanded={unfolded} onToggle={onToggle} section={section} />
    </section>
  )
}

/** Un dépôt découvert : son nom, et toute la ligne ouvre sa page sur github.com. */
function DiscoveryLine({ row }: { row: DiscoveryRow }) {
  const name = (
    <div className="row-text">
      <p className="row-name">{row.name}</p>
      <p className="row-detail">Premier commit sur ce dépôt</p>
    </div>
  )
  return (
    <li data-testid="github-discovery">
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

export function GithubCard({ date, today, state, health, calendarDay }: Props) {
  // Les dépôts dépliés, pour un jour : changer de jour replie, même en revenant.
  const [unfolded, setUnfolded] = useState<{ date: string; repos: string[] }>({ date: '', repos: [] })
  const { github, loadFailed } = state
  const open = unfolded.date === date ? unfolded.repos : []
  const toggle = (key: string) => setUnfolded({ date, repos: open.includes(key) ? open.filter((k) => k !== key) : [...open, key] })

  const active = github !== null && activeGithub(github)
  const lines = github !== null ? dayLines(github) : { known: 0, unknown: 0 }
  const summary = github !== null ? activitySummary(github) : ''
  const finds = github !== null ? discoveryRows(github) : []

  return (
    <section className="card card-github" aria-labelledby="titre-github" data-testid="github">
      <h2 id="titre-github" className="card-title card-title-github">
        GitHub
      </h2>

      {loadFailed && (
        <div className="inline-failure" role="status">
          <span>Impossible d'actualiser l'activité GitHub.</span>
          <button type="button" className="notice-action" onClick={() => void state.reload()}>
            Réessayer
          </button>
        </div>
      )}
      {githubSilent(health) && (
        <p className="inline-note" role="status" data-testid="github-alert">
          <strong>Le collecteur GitHub ne donne plus signe de vie.</strong> Des commits de ce jour peuvent manquer.
        </p>
      )}

      {!github && !loadFailed && <p className="empty">Chargement…</p>}
      {github && !active && <p className="empty">{githubEmptyText(date, today, health, calendarDay ? coveredValue(calendarDay.values) : undefined)}</p>}

      {active && (
        <>
          {github.commits > 0 && (
            <dl className="stats">
              <div>
                <dt>Commits</dt>
                <dd className="stat-value" data-testid="github-commits">
                  {github.commits}
                </dd>
              </div>
              <div>
                <dt>Lignes</dt>
                {lines.known > 0 ? (
                  <dd className="stat-value">
                    <span className="gh-day-lines" aria-hidden="true" data-testid="github-lines">
                      <span className="gh-add">{addedText(github.additions)}</span>
                      <span className="gh-del">{deletedText(github.deletions)}</span>
                    </span>
                    <span className="visually-hidden">{linesLabel(github.additions, github.deletions)}</span>
                  </dd>
                ) : (
                  <dd className="stat-value gh-lines-unknown" data-testid="github-lines">
                    inconnues
                  </dd>
                )}
                {lines.unknown > 0 && lines.known > 0 && (
                  <dd className="stat-note" data-testid="github-lines-unknown">
                    {unknownLinesText(lines.unknown)}
                  </dd>
                )}
              </div>
            </dl>
          )}
          {summary && (
            <p className="gh-summary" data-testid="github-summary">
              {summary}
            </p>
          )}

          {/* Un jour sans nouveau dépôt est la règle : pas de section du tout, plutôt qu'un « aucun ». */}
          {finds.length > 0 && (
            <section className="music-section" aria-labelledby="titre-decouvertes-github" data-testid="github-discoveries">
              <h3 id="titre-decouvertes-github" className="subsection-title">
                Découvertes
                <span className="subsection-count subsection-count-github">{finds.length}</span>
              </h3>
              <ul className="rows video-rows">
                {finds.map((row) => (
                  <DiscoveryLine key={row.key} row={row} />
                ))}
              </ul>
            </section>
          )}

          {github.repos.map((repo) => (
            <RepoSection key={repo.key} repo={repo} unfolded={open.includes(repo.key)} onToggle={() => toggle(repo.key)} />
          ))}
        </>
      )}
    </section>
  )
}
