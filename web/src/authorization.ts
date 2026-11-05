// Le retour de Google sur `/autorisation` : la page lit la réponse dans l'adresse, la transmet une fois
// au backend avec la session, puis efface le code de l'adresse. Le code ne s'affiche ni ne se journalise.

import type { AuthorizationCallback, CallbackResult } from './api'

/** La réponse de Google, telle que le backend l'attend ; `null` si l'adresse ne porte pas de réponse. */
export function callbackFrom(search: string): AuthorizationCallback | null {
  const query = new URLSearchParams(search)
  const state = query.get('state')
  const code = query.get('code')
  const error = query.get('error')
  if (!state) return null
  if (code && !error) return { state, code }
  if (error && !code) return { state, error }
  return null
}

/** Ce que la page dit. `failed` : le serveur n'a pas répondu, on peut réessayer. */
export type Outcome = 'received' | 'refused' | 'google_error' | 'expired' | 'invalid' | 'failed'

const OUTCOMES: Outcome[] = ['received', 'refused', 'google_error', 'expired', 'invalid', 'failed']

export function outcomeOf(callback: AuthorizationCallback, result: CallbackResult): Outcome {
  if (result !== 'received') return result
  if ('code' in callback) return 'received'
  return callback.error === 'access_denied' ? 'refused' : 'google_error'
}

/**
 * L'issue gardée dans l'entrée d'historique, à la place de la réponse : recharger la page la redit au
 * lieu de conclure à un lien expiré.
 */
export function storedOutcome(historyState: unknown): Outcome | null {
  const value = (historyState as { authorization?: unknown } | null)?.authorization
  return OUTCOMES.find((outcome) => outcome === value) ?? null
}

export const OUTCOME_TEXT: Record<Outcome, { title: string; text: string }> = {
  received: { title: 'Accord reçu.', text: 'La collecte YouTube reprend dans une minute.' },
  refused: {
    title: "Tu as refusé l'accord.",
    text: 'La collecte reste en attente. Pour la reprendre, redonne ton accord depuis la page État.',
  },
  google_error: { title: "Google n'a pas donné l'accord.", text: 'Relance depuis la page État.' },
  expired: { title: 'Ce lien a expiré.', text: 'Relance depuis la page État.' },
  invalid: { title: "Ce lien n'est pas valable.", text: 'Relance depuis la page État.' },
  failed: { title: "Impossible de transmettre l'accord.", text: 'Le serveur ne répond pas pour l’instant. Réessaie dans un moment.' },
}

/** L'issue est une réussite, une attente à relancer ou une panne. */
export const outcomeTone = (outcome: Outcome) => (outcome === 'received' ? 'ok' : outcome === 'failed' ? 'broken' : 'todo')
