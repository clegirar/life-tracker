import { describe, expect, it } from 'vitest'
import type { Cup } from './api'
import { failures, settle, unsettled, visibleCups, type Op, type OpStatus } from './reconcile'

const DATE = '2026-10-31'

const cup = (key: string, time = '10:00', source = 'webapp'): Cup => ({
  key,
  time,
  source,
  occurred_at: `${DATE}T${time}:00+01:00`,
})

const add = (key: string, status: OpStatus = 'sending', time = '11:00'): Op => ({
  date: DATE,
  time,
  status,
  record: {
    key,
    kind: 'coffee.cup',
    schema_version: 1,
    occurred_at: `${DATE}T${time}:00+01:00`,
    payload: {},
  },
})

const cancel = (target: string, status: OpStatus = 'sending', source = 'webapp'): Op => ({
  date: DATE,
  time: '10:00',
  status,
  record: {
    key: `cancel-${target}`,
    kind: 'record.cancel',
    schema_version: 1,
    occurred_at: `${DATE}T12:00:00+01:00`,
    payload: { source, key: target },
  },
})

const keys = (cups: Cup[]) => cups.map((c) => c.key)

describe('visibleCups', () => {
  it('ajoute un café en cours et le trie par heure', () => {
    expect(keys(visibleCups([cup('a', '12:00')], [add('b', 'sending', '11:00')], DATE))).toEqual(['b', 'a'])
  })

  it('ne compte pas deux fois un café que le serveur montre déjà', () => {
    expect(keys(visibleCups([cup('a')], [add('a', 'sent')], DATE))).toEqual(['a'])
  })

  it('ne montre pas un ajout en échec', () => {
    expect(visibleCups([], [add('a', 'failed')], DATE)).toEqual([])
  })

  it('ne montre un ajout que le jour auquel il appartient', () => {
    expect(visibleCups([], [add('a')], '2026-10-30')).toEqual([])
  })

  it('masque une cible annulée, y compris d’une autre source', () => {
    const server = [cup('a', '10:00', 'ios')]
    expect(visibleCups(server, [cancel('a', 'sending', 'ios')], DATE)).toEqual([])
    expect(visibleCups(server, [cancel('a', 'sent', 'ios')], DATE)).toEqual([])
    expect(visibleCups(server, [cancel('a', 'sent', 'webapp')], DATE)).toHaveLength(1)
  })

  it('rend le café si son annulation échoue', () => {
    expect(keys(visibleCups([cup('a')], [cancel('a', 'failed')], DATE))).toEqual(['a'])
  })

  it('ne fait pas réapparaître un café annulé que le worker traite après l’annulation', () => {
    const ops = [cancel('a', 'sent')]
    expect(visibleCups([], ops, DATE)).toEqual([])
    expect(visibleCups([cup('a')], ops, DATE)).toEqual([])
    expect(visibleCups([], ops, DATE)).toEqual([])
  })

  it('le nombre ne bouge pas pendant la réconciliation', () => {
    let ops = [add('a', 'sending')]
    expect(visibleCups([], ops, DATE)).toHaveLength(1)
    ops = [add('a', 'sent')]
    expect(visibleCups([], ops, DATE)).toHaveLength(1)
    const server = [cup('a', '11:00')]
    ops = settle(ops, DATE, server)
    expect(ops).toEqual([])
    expect(visibleCups(server, ops, DATE)).toHaveLength(1)
  })
})

describe('settle', () => {
  it('retire un ajout « en échec » que le serveur a pourtant reçu', () => {
    expect(settle([add('a', 'failed')], DATE, [cup('a')])).toEqual([])
  })

  it('garde ce que le serveur ne montre pas encore, et les annulations', () => {
    const ops = [add('b', 'sent'), cancel('a', 'sent')]
    expect(settle(ops, DATE, [cup('a')])).toEqual(ops)
  })
})

describe('unsettled', () => {
  it('attend un ajout reçu et pas encore visible', () => {
    expect(unsettled([], [add('a', 'sent')], DATE)).toHaveLength(1)
    expect(unsettled([], [add('a', 'sending')], DATE)).toHaveLength(0)
  })

  it('attend une annulation tant que sa cible est visible sur le serveur', () => {
    expect(unsettled([cup('a')], [cancel('a', 'sent')], DATE)).toHaveLength(1)
    expect(unsettled([], [cancel('a', 'sent')], DATE)).toHaveLength(0)
  })

  it('n’attend pas un ajout déjà annulé', () => {
    expect(unsettled([], [add('a', 'sent'), cancel('a', 'sent')], DATE)).toHaveLength(0)
  })
})

describe('failures', () => {
  it('signale un échec, sauf celui d’un ajout annulé entre-temps', () => {
    expect(failures([add('a', 'failed')])).toHaveLength(1)
    expect(failures([add('a', 'failed'), cancel('a', 'sent')])).toHaveLength(0)
    expect(failures([cancel('a', 'failed')])).toHaveLength(1)
  })
})
