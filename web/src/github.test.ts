import { describe, expect, it } from 'vitest'
import type { DayGithub, GithubRepo, SourcesHealth } from './api'
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
  githubUrl,
  linesLabel,
  repoRows,
  repoSummary,
  repoUrl,
  unknownLinesText,
} from './github'

const NNBSP = ' '

const MINE: GithubRepo = {
  key: 'R_kgDOFICTIF1',
  name: 'moi/depot-fictif',
  url: 'https://github.com/moi/depot-fictif',
  commits: [
    { sha: '0123456789abcdef', time: '21:15', title: 'Ajoute le compteur fictif', additions: 40, deletions: 2, url: 'https://github.com/moi/depot-fictif/commit/0123456789abcdef' },
    { sha: 'fedcba9876543210', time: '22:40', title: null, additions: null, deletions: null, url: null },
  ],
  additions: 40,
  deletions: 2,
  pull_requests: [
    { action: 'opened', number: 12, title: 'Compteur fictif', time: '21:30', url: 'https://github.com/moi/depot-fictif/pull/12', additions: null, deletions: null },
    { action: 'merged', number: 12, title: 'Compteur fictif', time: '22:00', url: 'https://github.com/moi/depot-fictif/pull/12', additions: 1_200, deletions: 30 },
  ],
  reviews: [],
  issues: [],
  comments: [],
}

const OTHER: GithubRepo = {
  key: 'R_kgDOFICTIF2',
  name: 'orga-fictive/autre-depot',
  url: 'https://github.com/orga-fictive/autre-depot',
  commits: [],
  additions: 0,
  deletions: 0,
  pull_requests: [],
  reviews: [{ state: 'changes_requested', number: 3, title: "La PR d'un autre", time: '11:05', url: 'https://github.com/orga-fictive/autre-depot/pull/3#pullrequestreview-1' }],
  issues: [{ number: 4, title: 'Un bogue fictif', time: '10:00', url: 'https://github.com/orga-fictive/autre-depot/issues/4' }],
  comments: [
    { target: 'issue', number: 4, title: 'Un bogue fictif', time: '10:30', url: 'https://github.com/orga-fictive/autre-depot/issues/4#issuecomment-1' },
    { target: 'commit', number: null, title: null, time: '12:00', url: 'https://evil.example/x' },
  ],
}

const DAY: DayGithub = {
  date: '2026-11-06',
  commits: 2,
  additions: 40,
  deletions: 2,
  pull_requests: 1,
  merged: 1,
  reviews: 1,
  issues: 1,
  comments: 2,
  repos: [MINE, OTHER],
  discoveries: [{ level: 'repo', key: 'R_kgDOFICTIF1', name: 'moi/depot-fictif' }],
}

const EMPTY: DayGithub = { ...DAY, commits: 0, additions: 0, deletions: 0, pull_requests: 0, merged: 0, reviews: 0, issues: 0, comments: 0, repos: [], discoveries: [] }

function health(overrides: { silent?: boolean; connected?: boolean; seen?: string | null } = {}): SourcesHealth {
  const { silent = false, connected = true, seen = '2026-10-20T08:00:00Z' } = overrides
  return {
    sources: connected ? [{ name: 'github', last_seen_at: seen, silence_threshold_seconds: 21_600, silent }] : [],
    metrics: [],
    errors: 0,
  }
}

describe('les lignes', () => {
  it('disent le plus et le vrai moins, avec les milliers', () => {
    expect(addedText(40)).toBe('+40')
    expect(deletedText(2)).toBe('−2')
    expect(addedText(12_345)).toBe(`+12${NNBSP}345`)
    expect(linesLabel(40, 1)).toBe('40 lignes ajoutées, 1 supprimée')
    expect(linesLabel(1, 0)).toBe('1 ligne ajoutée, 0 supprimée')
  })
  it('comptent les commits sans lignes à part, jamais comme zéro', () => {
    expect(dayLines(DAY)).toEqual({ known: 1, unknown: 1 })
    expect(unknownLinesText(1)).toBe('Lignes inconnues pour 1 commit.')
    expect(unknownLinesText(2)).toBe('Lignes inconnues pour 2 commits.')
  })
})

describe('le jour', () => {
  it('est actif dès une activité, même sans commit', () => {
    expect(activeGithub(DAY)).toBe(true)
    expect(activeGithub(EMPTY)).toBe(false)
    expect(activeGithub({ ...EMPTY, comments: 1 })).toBe(true)
  })
  it('résume le reste du jour en une phrase', () => {
    expect(activitySummary(DAY)).toBe('1 pull request ouverte · 1 fusionnée · 1 review · 1 issue · 2 commentaires')
    expect(activitySummary({ ...EMPTY, merged: 2 })).toBe('2 pull requests fusionnées')
    expect(activitySummary({ ...EMPTY, pull_requests: 3, reviews: 2 })).toBe('3 pull requests ouvertes · 2 reviews')
    expect(activitySummary(EMPTY)).toBe('')
  })
})

describe('les dépôts', () => {
  it('listent les commits, puis les pull requests, chacun avec son heure et ses lignes', () => {
    const rows = repoRows(MINE)
    expect(rows.map((row) => row.kind)).toEqual(['commit', 'commit', 'pull_request', 'merged'])
    expect(rows[0]).toMatchObject({
      name: 'Ajoute le compteur fictif',
      untitled: false,
      detail: '21:15 · 0123456',
      lines: { additions: 40, deletions: 2 },
      url: 'https://github.com/moi/depot-fictif/commit/0123456789abcdef',
    })
    // Un très gros commit : sans titre ni lignes, dit tel quel.
    expect(rows[1]).toMatchObject({ name: 'Commit fedcba9', untitled: true, lines: 'unknown', url: null })
    // Une ouverture n'a pas de lignes ; la fusion a les siennes.
    expect(rows[2]).toMatchObject({ detail: '21:30 · Pull request #12 ouverte', lines: null })
    expect(rows[3]).toMatchObject({ detail: '22:00 · Pull request #12 fusionnée', lines: { additions: 1_200, deletions: 30 } })
    expect(new Set(rows.map((row) => row.key)).size).toBe(rows.length)
  })
  it('disent reviews, issues et commentaires, et ne font un lien que vers github.com', () => {
    const rows = repoRows(OTHER)
    expect(rows.map((row) => row.detail)).toEqual([
      '11:05 · Review changements demandés #3',
      '10:00 · Issue #4 ouverte',
      "10:30 · Commentaire sur l'issue #4",
      '12:00 · Commentaire sur un commit',
    ])
    expect(rows[3]).toMatchObject({ name: 'Commit', untitled: true, url: null })
  })
  it('résument chaque dépôt sous son nom', () => {
    expect(repoSummary(MINE)).toBe('2 commits · +40 −2 · 1 pull request ouverte · 1 fusionnée')
    expect(repoSummary(OTHER)).toBe('1 review · 1 issue · 2 commentaires')
    // Aucun commit aux lignes connues : pas de « +0 −0 ».
    expect(repoSummary({ ...MINE, commits: [MINE.commits[1]], additions: 0, deletions: 0, pull_requests: [] })).toBe('1 commit')
  })
  it('ne fait de lien que vers une page de github.com', () => {
    expect(githubUrl('https://github.com/moi/depot-fictif/pull/12')).toBe('https://github.com/moi/depot-fictif/pull/12')
    for (const url of ['http://github.com/x', 'https://github.com.evil.example/x', 'https://user:pw@github.com/x', 'javascript:alert(1)', 'pas une url', null]) {
      expect(githubUrl(url), String(url)).toBeNull()
    }
    expect(repoUrl('moi/depot-fictif')).toBe('https://github.com/moi/depot-fictif')
    for (const name of ['', 'moi', 'moi/a/b', '../x', 'moi/..', 'moi/dépôt']) expect(repoUrl(name), name).toBeNull()
  })
})

describe('les découvertes', () => {
  it('listent les dépôts du premier commit, avec leur page', () => {
    expect(discoveryRows(DAY)).toEqual([{ key: 'R_kgDOFICTIF1', name: 'moi/depot-fictif', url: 'https://github.com/moi/depot-fictif' }])
    expect(discoveryRows(EMPTY)).toEqual([])
  })
})

describe('githubSilent', () => {
  it('ne regarde que les sources GitHub', () => {
    expect(githubSilent(health({ silent: true }))).toBe(true)
    expect(githubSilent(health())).toBe(false)
    expect(githubSilent(null)).toBe(false)
    expect(githubSilent({ sources: [{ name: 'steam', last_seen_at: null, silence_threshold_seconds: 1, silent: true }], metrics: [], errors: 0 })).toBe(false)
  })
})

describe('githubEmptyText', () => {
  const today = '2026-10-20'
  it('sans source, sans le mois, ou avant la première donnée', () => {
    expect(githubEmptyText('2026-10-14', today, null, 0)).toBe('Aucune activité GitHub reçue pour ce jour.')
    expect(githubEmptyText('2026-10-14', today, health(), undefined)).toBe('Aucune activité GitHub reçue pour ce jour.')
    expect(githubEmptyText('2026-10-14', today, health({ connected: false }), null)).toBe("GitHub n'est pas encore relié.")
    expect(githubEmptyText('2026-09-30', today, health(), null)).toBe('Pas de données GitHub à cette date.')
  })
  it('après le dernier passage du collecteur : on ne sait pas encore', () => {
    expect(githubEmptyText('2026-10-19', today, health({ seen: '2026-10-18T21:00:00Z' }), 0)).toMatch(/^Pas encore de données pour ce jour/)
    expect(githubEmptyText('2026-10-19', today, health({ seen: '2026-10-18T23:30:00Z' }), 0)).toBe('Rien sur GitHub hier.')
  })
  it('jusqu’au dernier passage : rien sur GitHub', () => {
    expect(githubEmptyText('2026-10-10', today, health(), 0)).toBe('Rien sur GitHub ce jour-là.')
    expect(githubEmptyText(today, today, health(), 0)).toBe("Rien sur GitHub aujourd'hui.")
  })
  it('lit la case des commits, ou des pull requests', () => {
    expect(coveredValue(undefined)).toBeUndefined()
    expect(coveredValue({ coffee: 2 })).toBeUndefined()
    expect(coveredValue({ 'code.commit': null, 'code.pull_request': null })).toBeNull()
    expect(coveredValue({ 'code.commit': 0, 'code.pull_request': null })).toBe(0)
    expect(coveredValue({ 'code.commit': null, 'code.pull_request': 1 })).toBe(1)
  })
})
