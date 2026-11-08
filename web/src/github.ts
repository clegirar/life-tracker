// Ce que la carte GitHub dit d'un jour : les commits et leurs lignes, puis, par dépôt, chaque commit,
// pull request, review, issue et commentaire. Un commit dont GitHub ne donne pas les lignes les a
// « inconnues », jamais zéro. Une journée sans données ne se confond jamais avec une journée sans code.

import type { DayGithub, GithubComment, GithubCommit, GithubIssue, GithubPullRequest, GithubRepo, GithubReview, RepoDiscovery, SourcesHealth } from './api'
import { plural } from './music'
import { addDays, parisDate } from './time'

// Un remplacement de token crée une nouvelle source : `github-2`.
const isGithub = (name: string) => /^github\b/.test(name)

const NUMBER = new Intl.NumberFormat('fr-FR', { maximumFractionDigits: 0 })
/** Le vrai signe moins, pas un trait d'union : il a la largeur du plus. */
const MINUS = '−'

/** Il s'est passé quelque chose sur GitHub ce jour-là. */
export const activeGithub = (day: DayGithub) => day.commits + day.pull_requests + day.merged + day.reviews + day.issues + day.comments > 0

/** Des lignes connues : « +40 », « −2 », avec l'espace fine des milliers. */
export const addedText = (count: number) => `+${NUMBER.format(count)}`
export const deletedText = (count: number) => `${MINUS}${NUMBER.format(count)}`

/** Pour un lecteur d'écran : « 40 lignes ajoutées, 2 supprimées ». */
export function linesLabel(additions: number, deletions: number): string {
  const added = `${NUMBER.format(additions)} ${additions > 1 ? 'lignes ajoutées' : 'ligne ajoutée'}`
  return `${added}, ${NUMBER.format(deletions)} ${deletions > 1 ? 'supprimées' : 'supprimée'}`
}

/** Les lignes d'un objet : connues, ou inconnues (jamais zéro), ou sans objet (une ouverture). */
export type Lines = { additions: number; deletions: number } | 'unknown' | null

function lines(additions: number | null, deletions: number | null): Lines {
  return additions === null || deletions === null ? 'unknown' : { additions, deletions }
}

/** Les lignes du jour : les commits qui les connaissent, et combien ne les donnent pas. */
export function dayLines(day: DayGithub): { known: number; unknown: number } {
  const unknown = day.repos.reduce((sum, repo) => sum + repo.commits.filter((commit) => lines(commit.additions, commit.deletions) === 'unknown').length, 0)
  return { known: day.commits - unknown, unknown }
}

/** « Lignes inconnues pour 1 commit » : un très gros commit dont GitHub tait les lignes. */
export const unknownLinesText = (count: number) => `Lignes inconnues pour ${plural(count, 'commit')}.`

/** Le reste du jour, en une phrase : « 1 pull request ouverte · 1 fusionnée · 2 reviews · 1 issue · 1 commentaire ». */
export function activitySummary(day: DayGithub): string {
  const parts: string[] = []
  const pr = (count: number) => (count > 1 ? `${count} pull requests` : `${count} pull request`)
  const agreed = (count: number, word: string) => `${word}${count > 1 ? 's' : ''}`
  if (day.pull_requests > 0) parts.push(`${pr(day.pull_requests)} ${agreed(day.pull_requests, 'ouverte')}`)
  if (day.merged > 0) parts.push(day.pull_requests > 0 ? `${day.merged} ${agreed(day.merged, 'fusionnée')}` : `${pr(day.merged)} ${agreed(day.merged, 'fusionnée')}`)
  if (day.reviews > 0) parts.push(plural(day.reviews, 'review'))
  if (day.issues > 0) parts.push(plural(day.issues, 'issue'))
  if (day.comments > 0) parts.push(plural(day.comments, 'commentaire'))
  return parts.join(' · ')
}

/** Seulement une page de github.com : jamais un lien venu d'ailleurs. */
export function githubUrl(url: string | null): string | null {
  if (url === null) return null
  try {
    const parsed = new URL(url)
    return parsed.protocol === 'https:' && parsed.hostname === 'github.com' && parsed.username === '' && parsed.password === '' ? parsed.href : null
  } catch {
    return null
  }
}

/** `propriétaire/nom` : la page du dépôt, sinon pas de lien. */
const REPO_NAME = /^[A-Za-z0-9](?:[A-Za-z0-9-]{0,38})\/[A-Za-z0-9._-]{1,100}$/
export const repoUrl = (name: string) => (REPO_NAME.test(name) && !name.endsWith('/.') && !name.endsWith('/..') ? `https://github.com/${name}` : null)

export type ActivityKind = 'commit' | 'pull_request' | 'merged' | 'review' | 'issue' | 'comment'

/** Une ligne d'un dépôt : un commit, une pull request, une review, une issue ou un commentaire. */
export interface ActivityRow {
  key: string
  kind: ActivityKind
  /** Le titre ; à défaut, ce qu'on sait de l'objet (« Commit 0123456 »), en gris. */
  name: string
  untitled: boolean
  /** « 21:15 · 0123456 », « 21:30 · Pull request #12 ouverte ». */
  detail: string
  lines: Lines
  url: string | null
}

const num = (number: number | null) => (number === null ? '' : ` #${number}`)
const shortSha = (sha: string) => sha.slice(0, 7)

const REVIEW_STATE: Record<NonNullable<GithubReview['state']>, string> = {
  approved: 'approuvée',
  changes_requested: 'changements demandés',
  commented: 'commentée',
  dismissed: 'écartée',
}

function commentTarget(comment: GithubComment): string {
  switch (comment.target) {
    case 'issue':
      return `Commentaire sur l'issue${num(comment.number)}`
    case 'pull_request':
      return `Commentaire sur la pull request${num(comment.number)}`
    case 'commit':
      return 'Commentaire sur un commit'
    case null:
      return 'Commentaire'
  }
}

function row(key: string, kind: ActivityKind, time: string, title: string | null, fallback: string, what: string, lines: Lines, url: string | null): ActivityRow {
  return {
    key,
    kind,
    name: title ?? fallback,
    untitled: title === null,
    detail: what ? `${time} · ${what}` : time,
    lines,
    url: githubUrl(url),
  }
}

const commitRow = (commit: GithubCommit, index: number): ActivityRow =>
  row(`commit:${commit.sha || index}`, 'commit', commit.time, commit.title, `Commit ${shortSha(commit.sha)}`.trim(), shortSha(commit.sha), lines(commit.additions, commit.deletions), commit.url)

function pullRequestRow(pr: GithubPullRequest, index: number): ActivityRow {
  const merged = pr.action === 'merged'
  const what = `Pull request${num(pr.number)} ${merged ? 'fusionnée' : 'ouverte'}`
  // Une ouverture n'a pas de lignes ; une fusion les a, ou les tait.
  return row(`pr:${pr.action}:${pr.number ?? index}:${pr.time}`, merged ? 'merged' : 'pull_request', pr.time, pr.title, `Pull request${num(pr.number)}`, what, merged ? lines(pr.additions, pr.deletions) : null, pr.url)
}

function reviewRow(review: GithubReview, index: number): ActivityRow {
  const state = review.state === null ? '' : ` ${REVIEW_STATE[review.state]}`
  return row(`review:${review.number ?? ''}:${review.time}:${index}`, 'review', review.time, review.title, `Pull request${num(review.number)}`, `Review${state}${num(review.number)}`, null, review.url)
}

const issueRow = (issue: GithubIssue, index: number): ActivityRow =>
  row(`issue:${issue.number ?? index}:${issue.time}`, 'issue', issue.time, issue.title, `Issue${num(issue.number)}`, `Issue${num(issue.number)} ouverte`, null, issue.url)

function commentRow(comment: GithubComment, index: number): ActivityRow {
  const fallback = comment.target === 'commit' ? 'Commit' : comment.target === 'pull_request' ? `Pull request${num(comment.number)}` : `Issue${num(comment.number)}`
  return row(`comment:${comment.time}:${index}`, 'comment', comment.time, comment.title, fallback, commentTarget(comment), null, comment.url)
}

/** Les lignes d'un dépôt : ses commits, puis ses pull requests, reviews, issues et commentaires, chacun par heure. */
export const repoRows = (repo: GithubRepo): ActivityRow[] => [
  ...repo.commits.map(commitRow),
  ...repo.pull_requests.map(pullRequestRow),
  ...repo.reviews.map(reviewRow),
  ...repo.issues.map(issueRow),
  ...repo.comments.map(commentRow),
]

/** Sous le nom du dépôt : « 3 commits · +40 −2 », « 1 review ». */
export function repoSummary(repo: GithubRepo): string {
  const parts: string[] = []
  if (repo.commits.length > 0) {
    parts.push(plural(repo.commits.length, 'commit'))
    const known = repo.commits.some((commit) => lines(commit.additions, commit.deletions) !== 'unknown')
    if (known) parts.push(`${addedText(repo.additions)} ${deletedText(repo.deletions)}`)
  }
  const opened = repo.pull_requests.filter((pr) => pr.action === 'opened').length
  const merged = repo.pull_requests.length - opened
  if (opened > 0) parts.push(`${opened > 1 ? `${opened} pull requests ouvertes` : '1 pull request ouverte'}`)
  if (merged > 0) parts.push(`${merged > 1 ? `${merged} fusionnées` : '1 fusionnée'}`)
  if (repo.reviews.length > 0) parts.push(plural(repo.reviews.length, 'review'))
  if (repo.issues.length > 0) parts.push(plural(repo.issues.length, 'issue'))
  if (repo.comments.length > 0) parts.push(plural(repo.comments.length, 'commentaire'))
  return parts.join(' · ')
}

/** Une ligne des découvertes : un dépôt sur lequel j'ai commité pour la première fois ce jour-là. */
export interface DiscoveryRow {
  key: string
  name: string
  url: string | null
}

/** Les dépôts découverts, dans l'ordre du serveur. Vide : la carte ne montre pas la section. */
export const discoveryRows = (day: DayGithub): DiscoveryRow[] =>
  day.discoveries.map((found: RepoDiscovery) => ({ key: found.key, name: found.name, url: repoUrl(found.name) }))

/** La source GitHub se tait : des commits de ce jour peuvent manquer. */
export const githubSilent = (health: SourcesHealth | null) =>
  health?.sources.some((source) => isGithub(source.name) && source.silent) ?? false

/**
 * Le texte d'une journée sans rien sur GitHub. `value` est la case du mois pour les commits (ou les
 * pull requests) : `null` avant la première donnée, `undefined` tant que le mois n'est pas lu. Le
 * collecteur passe tous les quarts d'heure : après son dernier passage, on ne sait pas encore.
 */
export function githubEmptyText(date: string, today: string, health: SourcesHealth | null, value: number | null | undefined): string {
  if (!health || value === undefined) return 'Aucune activité GitHub reçue pour ce jour.'
  const sources = health.sources.filter((source) => isGithub(source.name))
  if (sources.length === 0) return "GitHub n'est pas encore relié."
  if (value === null) return 'Pas de données GitHub à cette date.'
  const seen = sources
    .map((source) => source.last_seen_at)
    .filter((at): at is string => at !== null)
    .map((at) => parisDate(Date.parse(at)))
    .sort()
    .at(-1)
  if (seen === undefined || date > seen) return "Pas encore de données pour ce jour : le collecteur n'a pas donné signe de vie depuis."
  if (date === today) return "Rien sur GitHub aujourd'hui."
  if (date === addDays(today, -1)) return 'Rien sur GitHub hier.'
  return 'Rien sur GitHub ce jour-là.'
}

/** La case du mois qui dit si le jour est couvert : les commits, ou à défaut les pull requests. */
export function coveredValue(values: Record<string, number | null> | undefined): number | null | undefined {
  if (values === undefined) return undefined
  const commits = values['code.commit']
  const prs = values['code.pull_request']
  if (commits === undefined && prs === undefined) return undefined
  return commits ?? prs ?? null
}
