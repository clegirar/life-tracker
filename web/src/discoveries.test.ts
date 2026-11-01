import { describe, expect, it } from 'vitest'
import type { FirstListen } from './api'
import { cancelOp, discoveryDetail, markFailureText, markOp, settleOps, visibleFirstListens } from './discoveries'

const DATE = '2026-10-16'
const row = (over: Partial<FirstListen> = {}): FirstListen => ({
  level: 'album',
  item_key: 'a\tl',
  name: 'L',
  artist: 'A',
  known_marks: [],
  ...over,
})
const sent = <T extends { status: string }>(op: T) => ({ ...op, status: 'sent' as const })

describe('les enregistrements', () => {
  it('un marquage garde les noms affichés de son seul niveau', () => {
    const payload = (over: Partial<FirstListen>) => markOp(DATE, row(over)).records[0].payload
    expect(payload({ level: 'artist', name: 'A' })).toEqual({ level: 'artist', artist: 'A' })
    expect(payload({})).toEqual({ level: 'album', artist: 'A', album: 'L' })
    expect(payload({ level: 'track', name: 'T' })).toEqual({ level: 'track', artist: 'A', track: 'T' })
  })

  it('annuler vise tous les marquages, chacun avec sa source', () => {
    const op = cancelOp(DATE, row({ known_marks: [{ source: 'webapp', key: 'm1' }, { source: 'ios', key: 'm2' }] }))
    expect(op.records.map((r) => [r.kind, r.payload])).toEqual([
      ['record.cancel', { source: 'webapp', key: 'm1' }],
      ['record.cancel', { source: 'ios', key: 'm2' }],
    ])
    expect(new Set(op.records.map((r) => r.key)).size).toBe(2)
  })
})

describe('la réconciliation', () => {
  it('un marquage attend sa propre clé, pas un autre', () => {
    const op = sent(markOp(DATE, row()))
    const other = row({ known_marks: [{ source: 'webapp', key: 'autre' }] })
    expect(settleOps([op], DATE, [other])).toEqual([op])
    const mine = row({ known_marks: [{ source: 'webapp', key: op.keys[0] }] })
    expect(settleOps([op], DATE, [mine])).toEqual([])
  })

  it("une annulation attend que tous ses marquages aient disparu", () => {
    const marks = [{ source: 'webapp', key: 'm1' }, { source: 'webapp', key: 'm2' }]
    const op = sent(cancelOp(DATE, row({ known_marks: marks })))
    expect(settleOps([op], DATE, [row({ known_marks: [marks[1]] })])).toEqual([op])
    expect(settleOps([op], DATE, [row()])).toEqual([])
  })

  it("ne conclut ni avant l'accusé de réception, ni sur un autre jour", () => {
    const sending = markOp(DATE, row())
    const mine = row({ known_marks: [{ source: 'webapp', key: sending.keys[0] }] })
    expect(settleOps([sending], DATE, [mine])).toEqual([sending])
    expect(settleOps([sent(sending)], '2026-10-17', [])).toHaveLength(1)
  })

  it('un élément qui n\'est plus une première écoute ne fait plus attendre', () => {
    expect(settleOps([sent(markOp(DATE, row()))], DATE, [])).toEqual([])
  })
})

describe("ce que montre l'écran", () => {
  it('le geste se voit tout de suite, et un échec rend le serveur', () => {
    const op = markOp(DATE, row())
    const visible = (ops: typeof op[]) => visibleFirstListens([row()], ops, DATE)[0]
    expect(visible([])).toMatchObject({ known: false, pending: false })
    expect(visible([op])).toMatchObject({ known: true, pending: true })
    expect(visible([{ ...op, status: 'failed' }])).toMatchObject({ known: false, pending: false })
    expect(visibleFirstListens([row()], [op], '2026-10-17')[0].known).toBe(false)
  })

  it('une ligne marquée ne se dit plus découverte', () => {
    expect(discoveryDetail({ level: 'artist', artist: 'A' }, false)).toBe('Artiste découvert')
    expect(discoveryDetail({ level: 'album', artist: 'A' }, false)).toBe('Album découvert · A')
    expect(discoveryDetail({ level: 'track', artist: 'A' }, true)).toBe('Titre · A')
  })

  it("l'échec nomme le geste", () => {
    const op = markOp(DATE, row())
    expect(markFailureText(op)).toBe("Le marquage de « L » n'a pas été enregistré.")
    expect(markFailureText({ ...op, known: false, final: true })).toBe(
      "L'annulation du marquage de « L » a été refusée par le serveur.",
    )
  })
})
