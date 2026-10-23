import { flattenAccountTree, isPositiveI64Decimal, type AccountNode } from './account-tree.ts'

const maximum = BigInt('9223372036854775807')
function record(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('餘額回應格式不符')
  return value as Record<string, unknown>
}
function money(value: unknown): bigint {
  if (typeof value !== 'string' || !/^(0|[1-9]\d*)$/.test(value) || value.length > 19) throw new Error('金額必須是精確整數字串')
  const amount = BigInt(value)
  if (amount > maximum) throw new Error('金額超出範圍')
  return amount
}

/** A complete verified trial balance is the only source of displayed balances. */
export function accountBalances(value: unknown, tree: AccountNode[], asOf: string): Map<string, bigint> {
  const source = record(value)
  if (source.as_of !== asOf || !Array.isArray(source.rows) || source.rows.length > 5_000) throw new Error('餘額日期或筆數不符')
  const all = flattenAccountTree(tree, new Set())
  const accounts = new Map(all.map(row => [row.account.id, row.account]))
  const balances = new Map(all.map(row => [row.account.id, BigInt(0)]))
  const seen = new Set<string>()
  let debit = BigInt(0), credit = BigInt(0)
  for (const item of source.rows) {
    const row = record(item)
    if (typeof row.account_id !== 'string' || !isPositiveI64Decimal(row.account_id) || seen.has(row.account_id)) throw new Error('餘額科目識別不符')
    seen.add(row.account_id)
    const account = accounts.get(row.account_id)
    if (!account || account.kind !== 'posting' || row.account_name !== account.name || row.element !== account.element) throw new Error('餘額與科目資料不一致')
    const dr = money(row.debit_balance_minor), cr = money(row.credit_balance_minor)
    if ((dr !== BigInt(0) && cr !== BigInt(0)) ||
        money(row.debit_turnover_minor) - money(row.credit_turnover_minor) !== dr - cr) throw new Error('餘額無法核對')
    debit += dr; credit += cr
    balances.set(account.id, dr - cr)
  }
  if (debit !== money(source.total_debit_minor) || credit !== money(source.total_credit_minor) || debit !== credit) throw new Error('試算表不平衡')
  // Roll category totals from descendants without adding unrelated capacity metadata.
  for (let index = all.length - 1; index >= 0; index -= 1) {
    const account = all[index]!.account
    const net = balances.get(account.id)!
    if (net > maximum || net < -maximum) throw new Error('分類合計超出範圍')
    if (account.parentId) balances.set(account.parentId, balances.get(account.parentId)! + net)
  }
  return balances
}

export function formatBalance(value: bigint | undefined): string {
  if (value === undefined) return '—'
  const zero = BigInt(0)
  const absolute = value < zero ? -value : value
  const digits = absolute.toString().replace(/\B(?=(\d{3})+(?!\d))/g, ',')
  return value === zero ? '0' : `${value > zero ? '借' : '貸'} ${digits}`
}
