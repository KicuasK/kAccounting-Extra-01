import assert from 'node:assert/strict'
import test from 'node:test'

import {
  flattenAccountTree,
  isPositiveI64Decimal,
  parseAccountTree,
  segmentAccountName,
} from '../src/account-tree.ts'
import type { AccountNode } from '../src/account-tree.ts'

function account(overrides: Record<string, unknown> = {}) {
  return {
    id: '1',
    parent_id: null,
    name: '銀行 Bank 100',
    element: 'asset',
    kind: 'category',
    is_active: true,
    revision: '1',
    created_at: '2026-10-22T00:00:00Z',
    updated_at: '2026-10-22T00:00:00Z',
    ...overrides,
  }
}

test('keeps exact signed-64-bit identifiers as validated decimal strings', () => {
  assert.equal(isPositiveI64Decimal('1'), true)
  assert.equal(isPositiveI64Decimal('9223372036854775807'), true)
  assert.equal(isPositiveI64Decimal('9223372036854775808'), false)
  assert.equal(isPositiveI64Decimal(''), false)
  assert.equal(isPositiveI64Decimal('0'), false)
  assert.equal(isPositiveI64Decimal('01'), false)
  assert.equal(isPositiveI64Decimal('1.5'), false)
  assert.equal(isPositiveI64Decimal('-1'), false)
  assert.equal(isPositiveI64Decimal('1e3'), false)
  assert.equal(isPositiveI64Decimal(' 1'), false)
  assert.equal(isPositiveI64Decimal('１'), false)
  assert.equal(isPositiveI64Decimal('١'), false)
})

test('parses a valid hierarchy and preserves identifier strings exactly', () => {
  const parsed = parseAccountTree([
    {
      account: account(),
      children: [
        {
          account: account({
            id: '9223372036854775807',
            parent_id: '1',
            name: '活期存款 001',
            kind: 'posting',
          }),
          children: [],
        },
      ],
    },
  ])

  assert.equal(parsed[0]?.children[0]?.account.id, '9223372036854775807')
  assert.equal(parsed[0]?.children[0]?.account.parentId, '1')
})

test('rejects non-string, noncanonical, and out-of-range exact integer fields', () => {
  for (const id of [1, '0', '01', '9223372036854775808']) {
    assert.throws(
      () => parseAccountTree([{ account: account({ id }), children: [] }]),
      /id 必須/,
    )
  }

  for (const revision of [1, '0', '01', '9223372036854775808']) {
    assert.throws(
      () => parseAccountTree([{ account: account({ revision }), children: [] }]),
      /revision 必須/,
    )
  }
})

test('rejects duplicate ids and hierarchy mismatches', () => {
  assert.throws(
    () =>
      parseAccountTree([
        { account: account(), children: [] },
        { account: account(), children: [] },
      ]),
    /id 重複/,
  )

  assert.throws(
    () =>
      parseAccountTree([
        {
          account: account(),
          children: [
            {
              account: account({ id: '2', parent_id: null, kind: 'posting' }),
              children: [],
            },
          ],
        },
      ]),
    /階層不一致/,
  )

  assert.throws(
    () =>
      parseAccountTree([
        {
          account: account(),
          children: [
            {
              account: account({
                id: '2',
                parent_id: '1',
                element: 'liability',
                kind: 'posting',
              }),
              children: [],
            },
          ],
        },
      ]),
    /會計要素不一致/,
  )

  assert.throws(
    () =>
      parseAccountTree([
        {
          account: account(),
          children: [
            {
              account: account({ id: '2', parent_id: '01', kind: 'posting' }),
              children: [],
            },
          ],
        },
      ]),
    /parent_id 必須是正整數十進位字串/,
  )
})

test('rejects posting accounts with children and unsupported account values', () => {
  assert.throws(
    () =>
      parseAccountTree([
        {
          account: account({ kind: 'posting' }),
          children: [
            {
              account: account({ id: '2', parent_id: '1', kind: 'posting' }),
              children: [],
            },
          ],
        },
      ]),
    /過帳科目不可包含子科目/,
  )
  assert.throws(
    () => parseAccountTree([{ account: account({ element: 'other' }), children: [] }]),
    /element 不受支援/,
  )
  assert.throws(
    () => parseAccountTree([{ account: account({ kind: 'summary' }), children: [] }]),
    /kind 不受支援/,
  )
  assert.throws(
    () => parseAccountTree([{ account: account({ name: ' 前導空白' }), children: [] }]),
    /科目名稱格式/,
  )
  assert.throws(
    () => parseAccountTree([{ account: account({ is_active: 'true' }), children: [] }]),
    /is_active 必須是布林值/,
  )
  assert.throws(
    () =>
      parseAccountTree([
        { account: account({ name: '科'.repeat(121) }), children: [] },
      ]),
    /科目名稱格式/,
  )
})

test('rejects malformed node envelopes and parent identifier types', () => {
  assert.throws(() => parseAccountTree([null]), /必須是物件/)
  assert.throws(
    () => parseAccountTree([{ account: null, children: [] }]),
    /account 必須是物件/,
  )
  assert.throws(
    () => parseAccountTree([{ account: account(), children: null }]),
    /children 必須是陣列/,
  )
  assert.throws(
    () =>
      parseAccountTree([
        { account: account({ parent_id: 1, kind: 'posting' }), children: [] },
      ]),
    /parent_id 必須是字串或 null/,
  )
})

test('rejects cyclic response objects without recursing indefinitely', () => {
  const node: { account: Record<string, unknown>; children: unknown[] } = {
    account: account(),
    children: [],
  }
  node.children.push(node)
  assert.throws(() => parseAccountTree([node]), /循環參照/)
})

test('fails closed when a response exceeds the display depth boundary', () => {
  let node: { account: Record<string, unknown>; children: unknown[] } = {
    account: account({ id: '66', parent_id: '65', kind: 'posting' }),
    children: [],
  }

  for (let id = 65; id >= 1; id -= 1) {
    node = {
      account: account({
        id: String(id),
        parent_id: id === 1 ? null : String(id - 1),
      }),
      children: [node],
    }
  }

  assert.throws(() => parseAccountTree([node]), /顯示深度上限/)
})

test('fails closed when a response exceeds the display node boundary', () => {
  const nodes = Array.from({ length: 5_001 }, (_, index) => ({
    account: account({ id: String(index + 1) }),
    children: [],
  }))

  assert.throws(() => parseAccountTree(nodes), /顯示數量上限/)
})

test('accepts exact depth and node boundaries', () => {
  let chain: { account: Record<string, unknown>; children: unknown[] } = {
    account: account({ id: '65', parent_id: '64', kind: 'posting' }),
    children: [],
  }
  for (let id = 64; id >= 1; id -= 1) {
    chain = {
      account: account({
        id: String(id),
        parent_id: id === 1 ? null : String(id - 1),
      }),
      children: [chain],
    }
  }

  assert.equal(parseAccountTree([chain])[0]?.children.length, 1)

  const nodes = Array.from({ length: 5_000 }, (_, index) => ({
    account: account({ id: String(index + 1) }),
    children: [],
  }))
  assert.equal(parseAccountTree(nodes).length, 5_000)
})

test('rejects oversized sparse child collections before traversal', () => {
  assert.throws(
    () =>
      parseAccountTree([
        {
          account: account(),
          children: new Array(5_000),
        },
      ]),
    /顯示數量上限/,
  )
})

test('flattens expanded branches and omits descendants of collapsed branches', () => {
  const tree = parseAccountTree([
    {
      account: account(),
      children: [
        {
          account: account({ id: '2', parent_id: '1', kind: 'posting' }),
          children: [],
        },
      ],
    },
  ])

  assert.deepEqual(
    flattenAccountTree(tree, new Set()).map(({ account: item, depth }) => [item.id, depth]),
    [
      ['1', 0],
      ['2', 1],
    ],
  )
  assert.deepEqual(
    flattenAccountTree(tree, new Set(['1'])).map(({ account: item }) => item.id),
    ['1'],
  )
})

test('flattens a deeply nested validated shape without recursive calls', () => {
  const parsedAccount = parseAccountTree([
    { account: account({ kind: 'posting' }), children: [] },
  ])[0]!.account
  let node: AccountNode = { account: parsedAccount, children: [] }

  for (let id = 2; id <= 10_000; id += 1) {
    node = {
      account: { ...parsedAccount, id: String(id) },
      children: [node],
    }
  }

  const flattened = flattenAccountTree([node], new Set())
  assert.equal(flattened.length, 10_000)
  assert.equal(flattened.at(-1)?.depth, 9_999)
})

test('segments only Han characters into the account-name Kai font run', () => {
  assert.deepEqual(segmentAccountName('銀行-Bank 100元'), [
    { text: '銀行', script: 'han' },
    { text: '-Bank 100', script: 'system' },
    { text: '元', script: 'han' },
  ])
})

test('counts Unicode code points and rejects C0 and C1 controls in names', () => {
  const boundaryName = '🧮'.repeat(120)
  assert.equal(
    parseAccountTree([
      { account: account({ name: boundaryName, kind: 'posting' }), children: [] },
    ])[0]?.account.name,
    boundaryName,
  )

  assert.throws(
    () =>
      parseAccountTree([
        { account: account({ name: `${'🧮'.repeat(120)}A` }), children: [] },
      ]),
    /科目名稱格式/,
  )
  assert.throws(
    () => parseAccountTree([{ account: account({ name: '銀行\u0000' }), children: [] }]),
    /科目名稱格式/,
  )
  assert.throws(
    () => parseAccountTree([{ account: account({ name: '銀行\u0085' }), children: [] }]),
    /科目名稱格式/,
  )
})

test('matches backend Unicode edge-whitespace semantics exactly', () => {
  const edgeWhitespace = [
    '\u0020',
    '\u00a0',
    '\u1680',
    '\u2000',
    '\u2001',
    '\u2002',
    '\u2003',
    '\u2004',
    '\u2005',
    '\u2006',
    '\u2007',
    '\u2008',
    '\u2009',
    '\u200a',
    '\u2028',
    '\u2029',
    '\u202f',
    '\u205f',
    '\u3000',
  ]

  for (const whitespace of edgeWhitespace) {
    assert.throws(
      () =>
        parseAccountTree([
          { account: account({ name: `${whitespace}科目` }), children: [] },
        ]),
      /科目名稱格式/,
    )
    assert.throws(
      () =>
        parseAccountTree([
          { account: account({ name: `科目${whitespace}` }), children: [] },
        ]),
      /科目名稱格式/,
    )
  }

  for (const formatCharacter of ['\ufeff', '\u200b', '\u180e']) {
    const name = `${formatCharacter}科目${formatCharacter}`
    assert.equal(
      parseAccountTree([
        { account: account({ name, kind: 'posting' }), children: [] },
      ])[0]?.account.name,
      name,
    )
  }
})

test('preserves RTL and HTML-like account-name text without interpreting it', () => {
  const accountName = '<img src=x onerror=alert(1)> حساب 銀行 100'
  const parsed = parseAccountTree([
    { account: account({ name: accountName, kind: 'posting' }), children: [] },
  ])
  const segments = segmentAccountName(accountName)

  assert.equal(parsed[0]?.account.name, accountName)
  assert.equal(segments.map(({ text }) => text).join(''), accountName)
  assert.deepEqual(
    segments.filter(({ script }) => script === 'han'),
    [{ text: '銀行', script: 'han' }],
  )
})
