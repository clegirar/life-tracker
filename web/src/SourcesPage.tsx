// La page Sources : une fiche par tracker, puis les autres clés à garder en vie. Elle dit où vit chaque
// clé et comment la remplacer ; elle n'en montre jamais la valeur. Les échéances que le serveur connaît
// (la clé Garmin, l'accès Google) et l'état des sources viennent de la même lecture que la page État.

import type { ReactNode } from 'react'
import type { SourceStatus, SourcesStatus } from './api'
import { docText, frenchSpaces, keyExpiry, OTHER_KEYS, sheetState, sourceOf, TRACKERS, type KeyInfo } from './sources'
import { STATE_WORD } from './status'
import { Rich } from './StatusPage'

interface Props {
  /** Le bouton du tiroir, sur iPhone. */
  menu: ReactNode
  now: number
  data: SourcesStatus | null
  loadFailed: boolean
  onReload: () => void
}

/** Le texte d'une fiche, avec ses espaces insécables et son code en police fixe. */
function Text({ text }: { text: string }) {
  return <Rich text={frenchSpaces(text)} />
}

function Key({ info, source, now, id }: { info: KeyInfo; source: SourceStatus | null; now: number; id: string }) {
  const expiry = keyExpiry(info, source, now)
  // Repliée : le nom et l'échéance, ce qu'on cherche d'un coup d'œil ; le reste au toucher.
  return (
    <section className="source-key" aria-labelledby={id}>
      <h4 id={id} className="source-key-name">
        <Text text={info.name} />
      </h4>
      <dl className="source-facts">
        <div className="source-fact" data-warn={expiry.urgency ?? undefined} data-testid="key-expiry">
          <dt>Échéance</dt>
          <dd>
            <Text text={expiry.text} />
          </dd>
        </div>
      </dl>
      <details className="source-more">
        <summary className="source-more-toggle">Où elle vit, comment la remplacer</summary>
        <p className="source-key-role">
          <Text text={info.role} />
        </p>
        <dl className="source-facts">
          <div className="source-fact">
            <dt>Où</dt>
            <dd>
              <Text text={info.where} />
            </dd>
          </div>
          <div className="source-fact">
            <dt>Remplacer</dt>
            <dd>
              <Text text={info.replace} /> <span className="source-doc">
                <Text text={docText(info.doc)} />
              </span>
            </dd>
          </div>
        </dl>
      </details>
    </section>
  )
}

export function SourcesPage({ menu, now, data, loadFailed, onReload }: Props) {
  return (
    <main className="page status-page sources-page">
      {menu && <div className="topbar">{menu}</div>}
      <header className="header">
        <div className="header-text">
          <h1 className="title">Sources</h1>
        </div>
      </header>

      <p className="sources-lead">
        Comment chaque tracker reçoit ses données, et les clés à garder en vie. Aucune valeur ici&nbsp;: seulement où chaque clé vit, son échéance
        et comment la remplacer.
      </p>

      {loadFailed && (
        <div className="notice" role="alert">
          <span>{data ? "Impossible d'actualiser les échéances." : 'Impossible de lire les échéances : seules les fiches s’affichent.'}</span>
          <button type="button" className="notice-action" onClick={onReload}>
            Réessayer
          </button>
        </div>
      )}

      <section className="sources-section" aria-labelledby="sources-trackers">
        <h2 id="sources-trackers" className="sources-heading">
          Trackers
        </h2>
        <ul className="status-cards">
          {TRACKERS.map((sheet) => {
            const source = sourceOf(sheet, data)
            const state = sheetState(source, now)
            const titleId = `fiche-${sheet.id}`
            return (
              <li key={sheet.id}>
                <article className="card status-card source-card" data-state={state ?? undefined} data-tracker={sheet.id} aria-labelledby={titleId}>
                  <header className="status-head">
                    <h3 id={titleId} className="status-name">
                      {sheet.title}
                    </h3>
                    {state && (
                      <span className="status-badge" data-state={state} data-testid="status-word">
                        <span className="status-badge-dot" aria-hidden="true" />
                        {STATE_WORD[state]}
                      </span>
                    )}
                  </header>
                  <p className="source-arrival">
                    <Text text={sheet.arrival} />
                  </p>
                  {data && !source && <p className="status-hint">Aucune source active sur le serveur pour l'instant.</p>}
                  <div className="source-keys">
                    {sheet.keys.map((key, index) => (
                      <Key key={key.name} info={key} source={source} now={now} id={`${titleId}-cle-${index}`} />
                    ))}
                  </div>
                </article>
              </li>
            )
          })}
        </ul>
      </section>

      <section className="sources-section" aria-labelledby="sources-autres">
        <h2 id="sources-autres" className="sources-heading">
          Autres clés à gérer
        </h2>
        <p className="sources-lead">Elles ne collectent rien, mais sans elles, plus rien ne se déploie, ne se lit ni ne se signe.</p>
        <ul className="status-cards">
          {OTHER_KEYS.map((group) => {
            const titleId = `cles-${group.id}`
            return (
              <li key={group.id}>
                <article className="card status-card source-card" data-group={group.id} aria-labelledby={titleId}>
                  <header className="status-head">
                    <h3 id={titleId} className="status-name">
                      {group.title}
                    </h3>
                  </header>
                  <div className="source-keys">
                    {group.keys.map((key, index) => (
                      <Key key={key.name} info={key} source={null} now={now} id={`${titleId}-${index}`} />
                    ))}
                  </div>
                </article>
              </li>
            )
          })}
        </ul>
      </section>

      <p className="sources-note">
        Une échéance passe en orange 14 jours avant (2 jours pour l'accord Google, qui ne dure que 7 jours), comme sur la page État, puis en
        rouge une fois passée. Les marches à suivre complètes sont dans <code>docs/deploiement.md</code>.
      </p>
    </main>
  )
}
