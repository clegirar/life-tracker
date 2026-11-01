// Les marquages « déjà connu » : ce que l'écran montre pendant qu'un geste traverse le journal et le
// worker. Le serveur fait foi dès qu'il montre le résultat ; une clé précise le dit, pas une supposition.

import { newKey, type FirstListen, type OutgoingRecord } from './api'
import { toRfc3339 } from './time'

export interface MarkOp {
  id: string
  date: string
  name: string
  /** L'état voulu : marqué, ou plus marqué. */
  known: boolean
  /** Le marquage, ou une annulation par marquage existant. */
  records: OutgoingRecord[]
  /** Les marquages attendus, ou ceux qui doivent disparaître. */
  keys: string[]
  status: 'sending' | 'sent' | 'failed'
  final?: boolean
}

export const itemId = (row: Pick<FirstListen, 'level' | 'item_key'>) => `${row.level}/${row.item_key}`

/** Les noms tels qu'affichés : le serveur en retrouve la clé, comme pour une écoute. */
function markPayload(row: FirstListen): Record<string, string> {
  switch (row.level) {
    case 'artist':
      return { level: 'artist', artist: row.name }
    case 'album':
      return { level: 'album', artist: row.artist, album: row.name }
    case 'track':
      return { level: 'track', artist: row.artist, track: row.name }
  }
}

export function markOp(date: string, row: FirstListen): MarkOp {
  const key = newKey()
  return {
    id: itemId(row),
    date,
    name: row.name,
    known: true,
    keys: [key],
    status: 'sending',
    records: [
      {
        key,
        kind: 'music.mark_known',
        schema_version: 1,
        occurred_at: toRfc3339(Date.now()),
        payload: markPayload(row),
      },
    ],
  }
}

/** Tous les marquages de l'élément : un seul restant le garderait « connu ». */
export function cancelOp(date: string, row: FirstListen): MarkOp {
  return {
    id: itemId(row),
    date,
    name: row.name,
    known: false,
    keys: row.known_marks.map((mark) => mark.key),
    status: 'sending',
    records: row.known_marks.map((mark) => ({
      key: newKey(),
      kind: 'record.cancel',
      schema_version: 1,
      occurred_at: toRfc3339(Date.now()),
      payload: { source: mark.source, key: mark.key },
    })),
  }
}

/** Retire ce que le serveur montre déjà. Un élément disparu de la liste n'a plus rien à attendre. */
export function settleOps(ops: MarkOp[], date: string, rows: FirstListen[]): MarkOp[] {
  return ops.filter((op) => {
    if (op.date !== date || op.status !== 'sent') return true
    const row = rows.find((other) => itemId(other) === op.id)
    if (!row) return false
    const present = row.known_marks.map((mark) => mark.key)
    return op.known
      ? !op.keys.every((key) => present.includes(key))
      : op.keys.some((key) => present.includes(key))
  })
}

export interface VisibleFirstListen extends FirstListen {
  known: boolean
  pending: boolean
}

/** Le geste se voit tout de suite ; un échec rend l'état du serveur, et le dit à part. */
export function visibleFirstListens(rows: FirstListen[], ops: MarkOp[], date: string): VisibleFirstListen[] {
  return rows.map((row) => {
    const op = ops.find((other) => other.date === date && other.id === itemId(row) && other.status !== 'failed')
    return { ...row, known: op ? op.known : row.known_marks.length > 0, pending: op !== undefined }
  })
}

const LEVELS = { artist: 'Artiste', album: 'Album', track: 'Titre' }
const DISCOVERED = { artist: 'Artiste découvert', album: 'Album découvert', track: 'Titre découvert' }

/** Un élément marqué n'est plus une découverte : la ligne cesse de le prétendre. */
export function discoveryDetail(row: Pick<FirstListen, 'level' | 'artist'>, known: boolean): string {
  const label = known ? LEVELS[row.level] : DISCOVERED[row.level]
  return row.level === 'artist' ? label : `${label} · ${row.artist}`
}

export function markFailureText(op: MarkOp): string {
  const what = op.known ? 'Le marquage' : "L'annulation du marquage"
  const e = op.known ? '' : 'e'
  return op.final
    ? `${what} de « ${op.name} » a été refusé${e} par le serveur.`
    : `${what} de « ${op.name} » n'a pas été enregistré${e}.`
}
