import assert from 'node:assert/strict'
import test from 'node:test'
import { accountView } from '../src/account-view.ts'
import { accountBalances, formatBalance } from '../src/account-balances.ts'
import type { AccountNode, AccountingElement } from '../src/account-tree.ts'

function node(id: string, name: string, element: AccountingElement, children: AccountNode[] = [], parentId: string | null = null, active = true): AccountNode {
  return { account: { id, name, element, kind: children.length ? 'category' : 'posting', parentId, isActive: active, revision: '1', createdAt: 'now', updatedAt: 'now' }, children }
}
const tree = [node('1', '資產 Assets', 'asset', [node('2', '現金 Cash', 'asset', [], '1'), node('3', '停用 Cash', 'asset', [], '1', false)]), node('4', '權益', 'equity')]
const filter = { query: '', element: '' as const, inactive: false }
const row = (id: string, name: string, element: string, debit: string, credit: string) => ({ account_id: id, account_name: name, element, debit_turnover_minor: debit, credit_turnover_minor: credit, debit_balance_minor: debit, credit_balance_minor: credit })
const trial = () => ({ as_of: '2026-10-22', rows: [row('2', '現金 Cash', 'asset', '9007199254740993', '0'), row('4', '權益', 'equity', '0', '9007199254740993')], total_debit_minor: '9007199254740993', total_credit_minor: '9007199254740993' })

test('active view preserves sibling order and respects collapsed ancestors', () => {
  assert.deepEqual(accountView(tree, new Set(), filter).rows.map(row => row.account.id), ['1', '2', '4'])
  assert.deepEqual(accountView(tree, new Set(['1']), filter).rows.map(row => row.account.id), ['1', '4'])
})
test('search includes full ancestor path and reveals matches below collapsed nodes', () => {
  const view = accountView(tree, new Set(['1']), { ...filter, query: 'assets' })
  assert.deepEqual(view.rows.map(row => row.account.id), ['1', '2'])
  assert.deepEqual(view.paths.get('2')!.map(node => node.account.name), ['資產 Assets', '現金 Cash'])
})
test('inactive view retains active ancestors as context without counting them as matches', () => {
  const view = accountView(tree, new Set(['1']), { ...filter, inactive: true })
  assert.deepEqual([...view.matchingIds], ['3'])
  assert.deepEqual(view.rows.map(row => row.account.id), ['1', '3'])
})
test('element and trimmed search filters intersect without modifying source', () => {
  const before = structuredClone(tree)
  assert.deepEqual(accountView(tree, new Set(), { ...filter, query: ' CASH ', element: 'asset' }).rows.map(row => row.account.id), ['1', '2'])
  assert.equal(accountView(tree, new Set(), { ...filter, query: 'Cash', element: 'equity' }).rows.length, 0)
  assert.deepEqual(tree, before)
})
test('balances preserve values beyond Number precision and roll up category descendants', () => {
  const balances = accountBalances(trial(), tree, '2026-10-22')
  assert.equal(formatBalance(balances.get('1')), '借 9,007,199,254,740,993')
  assert.equal(formatBalance(balances.get('4')), '貸 9,007,199,254,740,993')
  assert.equal(formatBalance(balances.get('3')), '0')
  assert.equal(formatBalance(undefined), '—')
})
test('wrong dates, duplicate/unknown accounts and metadata mismatch never produce balances', () => {
  assert.throws(() => accountBalances(trial(), tree, '2026-10-23'))
  for (const change of [
    (v: ReturnType<typeof trial>) => { v.rows.push(v.rows[0]!) },
    (v: ReturnType<typeof trial>) => { v.rows[0]!.account_id = '999' },
    (v: ReturnType<typeof trial>) => { v.rows[0]!.account_name = 'unmatched' },
    (v: ReturnType<typeof trial>) => { v.rows[0]!.element = 'liability' },
    (v: ReturnType<typeof trial>) => { v.rows[0]!.account_id = '1'; v.rows[0]!.account_name = '資產 Assets' },
  ]) {
    const value = trial(); change(value)
    assert.throws(() => accountBalances(value, tree, '2026-10-22'))
  }
})
test('rounded numbers, noncanonical integers, overflow and inconsistent totals are refused', () => {
  for (const amount of [9007199254740992, '01', '-1', '1e3', '9223372036854775808', null]) {
    const value = trial() as unknown as { rows: Record<string, unknown>[] }
    value.rows[0]!.debit_balance_minor = amount
    assert.throws(() => accountBalances(value, tree, '2026-10-22'))
  }
  const value = trial(); value.total_credit_minor = '1'
  assert.throws(() => accountBalances(value, tree, '2026-10-22'))
})
test('two-sided balances and turnover mismatch are rejected; a complete empty report yields zero', () => {
  const value = trial(); value.rows[0]!.credit_balance_minor = '1'
  assert.throws(() => accountBalances(value, tree, '2026-10-22'))
  value.rows[0]!.credit_balance_minor = '0'; value.rows[0]!.debit_turnover_minor = '1'
  assert.throws(() => accountBalances(value, tree, '2026-10-22'))
  assert.equal(formatBalance(accountBalances({ as_of: '2026-10-22', rows: [], total_debit_minor: '0', total_credit_minor: '0' }, tree, '2026-10-22').get('2')), '0')
})
