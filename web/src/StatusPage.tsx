// La page d'état des trackers : une carte par source, son état en un mot, ses dernières données et,
// quand il le faut, la marche à suivre et son bouton. Elle affiche et rappelle ; elle ne reçoit aucune clé.

import type { ReactNode } from 'react'
import type { SourcesStatus } from './api'
import { codeParts, sourceCard, STATE_WORD, type Card, type Guidance } from './status'

interface Props {
  /** Le bouton du tiroir, sur iPhone. */
  menu: ReactNode
  now: number
  data: SourcesStatus | null
  loadFailed: boolean
  onReload: () => void
}

/** Une étape : du texte, et les fichiers ou commandes en police fixe. */
export function Rich({ text }: { text: string }) {
  return codeParts(text).map((part, index) => (part.code ? <code key={index}>{part.text}</code> : <span key={index}>{part.text}</span>))
}

function Help({ guidance, id }: { guidance: Guidance; id: string }) {
  return (
    <section className="status-help" aria-labelledby={id}>
      <h3 id={id} className="status-help-title">
        Que faire
      </h3>
      <p className="status-help-lead">
        <Rich text={guidance.title} />
      </p>
      {guidance.steps.length > 0 && (
        <ol className="status-steps">
          {guidance.steps.map((step) => (
            <li key={step}>
              <Rich text={step} />
            </li>
          ))}
        </ol>
      )}
      {guidance.note && (
        <p className="status-help-note">
          <Rich text={guidance.note} />
        </p>
      )}
      {/* Dans l'onglet même : Google revient sur `/autorisation`, qui transmet sa réponse. */}
      {guidance.action && (
        <a className="primary status-action" href={guidance.action.href} rel="noreferrer">
          {guidance.action.label}
        </a>
      )}
    </section>
  )
}

function SourceCard({ card }: { card: Card }) {
  const titleId = `etat-${card.name}`
  return (
    <article className="card status-card" data-state={card.state} data-source={card.name} aria-labelledby={titleId}>
      <header className="status-head">
        <h2 id={titleId} className="status-name">
          {card.title}
        </h2>
        <span className="status-badge" data-state={card.state} data-testid="status-word">
          <span className="status-badge-dot" aria-hidden="true" />
          {STATE_WORD[card.state]}
        </span>
      </header>
      <dl className="status-lines">
        {card.lines.map((line) => (
          <div key={line.label} className="status-line" data-warn={line.warn}>
            <dt>{line.label}</dt>
            <dd>{line.value}</dd>
          </div>
        ))}
      </dl>
      {card.hint && <p className="status-hint">{card.hint}</p>}
      {card.guidance && <Help guidance={card.guidance} id={`${titleId}-aide`} />}
    </article>
  )
}

export function StatusPage({ menu, now, data, loadFailed, onReload }: Props) {
  const cards = data?.sources.map((source) => sourceCard(source, now)) ?? null

  return (
    <main className="page status-page">
      {menu && <div className="topbar">{menu}</div>}
      <header className="header">
        <div className="header-text">
          <h1 className="title">État</h1>
        </div>
      </header>

      {loadFailed && (
        <div className="notice" role="alert">
          <span>{cards ? "Impossible d'actualiser l'état des trackers." : "Impossible de lire l'état des trackers."}</span>
          <button type="button" className="notice-action" onClick={onReload}>
            Réessayer
          </button>
        </div>
      )}

      {cards && cards.length === 0 && <p className="status-quiet">Aucun tracker à surveiller pour l'instant.</p>}

      {cards && cards.length > 0 && (
        <ul className="status-cards">
          {cards.map((card) => (
            <li key={card.name}>
              <SourceCard card={card} />
            </li>
          ))}
        </ul>
      )}
    </main>
  )
}
