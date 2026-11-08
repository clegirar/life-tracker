// La page Sources : une fiche par tracker, puis les autres clés à garder en vie. De chaque clé, seulement
// son nom et son échéance, jamais sa valeur. Les échéances que le serveur connaît (la clé Garmin, l'accès
// Google) et l'état des sources viennent de la même lecture que la page État.

import type { ReactNode } from 'react'
import { useSourcesStatus } from './useSourcesStatus'
import type { SourceStatus } from './api'
import { frenchSpaces, keyExpiry, OTHER_KEYS, sheetState, sourceOf, TRACKERS, type KeyInfo } from './sources'
import { STATE_WORD } from './status'
import { Rich } from './StatusPage'

interface Props {
  /** Le bouton du tiroir, sur iPhone. */
  menu: ReactNode
  now: number
}

/** Le texte d'une fiche, avec ses espaces insécables et son code en police fixe. */
function Text({ text }: { text: string }) {
  return <Rich text={frenchSpaces(text)} />
}

function Key({ info, source, now, id }: { info: KeyInfo; source: SourceStatus | null; now: number; id: string }) {
  const expiry = keyExpiry(info, source, now)
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
    </section>
  )
}

export function SourcesPage({ menu, now }: Props) {
  // Chaque ouverture de la page relit l'état, puis chaque minute tant qu'elle est ouverte.
  const { status: data, loadFailed, reload } = useSourcesStatus(true, true)
  const onReload = () => void reload()
  return (
    <main className="page status-page sources-page">
      {menu && <div className="topbar">{menu}</div>}
      <header className="header">
        <div className="header-text">
          <h1 className="title">Sources</h1>
        </div>
      </header>

      <p className="sources-lead">
        Comment chaque tracker reçoit ses données, et les clés à garder en vie. Aucune valeur ici&nbsp;: seulement le nom de chaque clé et son
        échéance.
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
        rouge une fois passée.
      </p>
    </main>
  )
}
