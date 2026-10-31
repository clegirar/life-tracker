// Ce que l'écran montre : les cafés du serveur, corrigés par les écritures de cette page.
// Le worker interprète de façon asynchrone ; ces fonctions pures garantissent que le nombre ne
// recule pas et ne contredit pas ce que je viens de faire.

import type { Cup, OutgoingRecord } from './api'

export type OpStatus = 'sending' | 'sent' | 'failed'

export interface Op {
  record: OutgoingRecord
  /** Le jour auquel l'écriture appartient, et l'heure qu'elle affiche. */
  date: string
  time: string
  status: OpStatus
  /** Le serveur a refusé : la renvoyer ne servirait à rien. */
  final?: boolean
}

const id = (source: string, key: string) => `${source}\n${key}`

export const isAdd = (op: Op) => op.record.kind === 'coffee.cup'

function target(op: Op): string {
  return id(op.record.payload.source, op.record.payload.key)
}

/**
 * Une annulation qui n'a pas échoué masque sa cible pour toute la vie de la page, même si le worker
 * n'a pas fini ou s'il traite le café après l'annulation : une annulation est définitive.
 */
function cancelled(ops: Op[]): Set<string> {
  return new Set(ops.filter((op) => !isAdd(op) && op.status !== 'failed').map(target))
}

export function visibleCups(server: Cup[], ops: Op[], date: string): Cup[] {
  const hidden = cancelled(ops)
  const known = new Set(server.map((cup) => id(cup.source, cup.key)))
  const pending: Cup[] = ops
    .filter((op) => isAdd(op) && op.date === date && op.status !== 'failed')
    .filter((op) => !known.has(id('webapp', op.record.key)))
    .map((op) => ({
      time: op.time,
      occurred_at: op.record.occurred_at,
      source: 'webapp',
      key: op.record.key,
    }))
  return [...server, ...pending]
    .filter((cup) => !hidden.has(id(cup.source, cup.key)))
    .sort((a, b) => Date.parse(a.occurred_at) - Date.parse(b.occurred_at))
}

/**
 * Retire les ajouts que le serveur montre désormais, même « en échec » : une réponse perdue n'empêche
 * pas l'écriture d'avoir eu lieu. Les annulations restent, car elles masquent leur cible.
 */
export function settle(ops: Op[], date: string, server: Cup[]): Op[] {
  const known = new Set(server.map((cup) => id(cup.source, cup.key)))
  return ops.filter((op) => !(isAdd(op) && op.date === date && known.has(id('webapp', op.record.key))))
}

/** Les écritures reçues par le serveur dont l'effet n'est pas encore visible dans sa réponse. */
export function unsettled(server: Cup[] | undefined, ops: Op[], date: string): Op[] {
  const hidden = cancelled(ops)
  const known = new Set((server ?? []).map((cup) => id(cup.source, cup.key)))
  return ops.filter((op) => {
    if (op.status !== 'sent' || op.date !== date) return false
    return isAdd(op)
      ? !hidden.has(id('webapp', op.record.key))
      : known.has(target(op))
  })
}

/** Un café dont l'ajout a été annulé n'a plus d'échec à signaler. */
export function failures(ops: Op[]): Op[] {
  const hidden = cancelled(ops)
  return ops.filter(
    (op) => op.status === 'failed' && !(isAdd(op) && hidden.has(id('webapp', op.record.key))),
  )
}
