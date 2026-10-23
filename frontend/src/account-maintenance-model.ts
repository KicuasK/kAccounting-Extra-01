import { isPositiveI64Decimal } from './account-tree.ts'
import type { AccountRecord, AccountingElement, AccountKind } from './account-tree.ts'

export const elementLabels: Record<AccountingElement, string> = { asset: '資產', liability: '負債', equity: '權益', income: '收益', expense: '費損' }
export function accountPathLabels(accounts: AccountRecord[]): Map<string, string> {
  const byId = new Map(accounts.map(account => [account.id, account]))
  const result = new Map<string, string>()
  for (const account of accounts) {
    const names: string[] = [], seen = new Set<string>()
    let node: AccountRecord | undefined = account
    while (node) {
      if (seen.has(node.id)) throw new Error('科目階層無效，請重新載入。')
      seen.add(node.id); names.unshift(node.name)
      if (node.parentId !== null && !byId.has(node.parentId)) throw new Error('科目階層不完整，請重新載入。')
      node = node.parentId === null ? undefined : byId.get(node.parentId)
    }
    result.set(account.id, `${elementLabels[account.element]} / ${names.join(' / ')}`)
  }
  return result
}
export function accountFailureRequiresReload(error: { code: string; status?: number | null } | null): boolean {
  if (error === null || ['account_revision_conflict', 'account_not_found', 'selection_changed'].includes(error.code)) return true
  if (error.status !== undefined && error.status !== null) return !(error.status >= 400 && error.status < 500)
  // Known domain validation responses are definite refusals even on older transports.
  return !['duplicate_account_name', 'invalid_account_name', 'account_rule_violation'].includes(error.code)
}
export type AccountChange =
  | { operation: 'rename'; name: string }
  | { operation: 'move'; parent_id: string | null }
  | { operation: 'set_active'; is_active: boolean }
  | { operation: 'change_kind'; kind: AccountKind }
  | { operation: 'change_element'; element: AccountingElement }
export interface AccountCommand { path: string; method: 'POST' | 'PATCH' | 'DELETE'; payload: unknown }
export function normalizeAccountName(value: string): string {
  const name = value.replace(/^\p{White_Space}+|\p{White_Space}+$/gu, '')
  if (!name || Array.from(name).length > 120 || /[\u0000-\u001f\u007f-\u009f]/.test(name)) throw new Error('名稱須為 1 至 120 字，且不可含控制字元。')
  return name
}
function exact(value: string): string { if (!isPositiveI64Decimal(value)) throw new Error('科目識別或版本無效，請重新載入。'); return value }
export function eligibleParents(accounts: AccountRecord[], selected?: AccountRecord): AccountRecord[] {
  const excluded = new Set<string>(selected ? [selected.id] : [])
  for (let count = 0; count < accounts.length; count++) {
    let changed = false
    for (const account of accounts) if (account.parentId !== null && excluded.has(account.parentId) && !excluded.has(account.id)) { excluded.add(account.id); changed = true }
    if (!changed) break
  }
  return accounts.filter(account => account.kind === 'category' && !excluded.has(account.id) && (!selected || account.element === selected.element))
}
export function createAccountCommand(name: string, element: AccountingElement, kind: AccountKind, parentId: string | null): AccountCommand {
  if (!(element in elementLabels) || !['posting', 'category'].includes(kind)) throw new Error('請選擇有效的科目類別。')
  return { path: '/api/accounts', method: 'POST', payload: { name: normalizeAccountName(name), element, kind, parent_id: parentId === null ? null : exact(parentId) } }
}
export function changeAccountCommand(account: AccountRecord, change: AccountChange): AccountCommand {
  const adjusted = change.operation === 'rename' ? { ...change, name: normalizeAccountName(change.name) } : change
  if (adjusted.operation === 'move' && adjusted.parent_id !== null) exact(adjusted.parent_id)
  return { path: `/api/accounts/${exact(account.id)}`, method: 'PATCH', payload: { ...adjusted, expected_revision: exact(account.revision) } }
}
export function deleteAccountCommand(account: AccountRecord): AccountCommand {
  return { path: `/api/accounts/${exact(account.id)}`, method: 'DELETE', payload: { expected_revision: exact(account.revision) } }
}
export function validateAccountCommandResponse(command: AccountCommand, value: unknown, before?: AccountRecord, existingIds: ReadonlySet<string> = new Set()): void {
  const refuse = (): never => { throw new Error('無法核對科目寫入結果；請重新載入確認。') }
  if (command.method === 'DELETE') { if (value !== null) refuse(); return }
  if (!value || typeof value !== 'object' || Array.isArray(value)) refuse()
  const row = value as Record<string, unknown>
  for (const key of ['id', 'revision']) if (typeof row[key] !== 'string' || !isPositiveI64Decimal(row[key] as string)) refuse()
  if (typeof row.name !== 'string' || normalizeAccountName(row.name) !== row.name ||
      typeof row.element !== 'string' || !Object.prototype.hasOwnProperty.call(elementLabels, row.element) ||
      !['category', 'posting'].includes(row.kind as string) || typeof row.is_active !== 'boolean' ||
      !(row.parent_id === null || (typeof row.parent_id === 'string' && isPositiveI64Decimal(row.parent_id))) ||
      typeof row.created_at !== 'string' || !row.created_at || typeof row.updated_at !== 'string' || !row.updated_at) refuse()
  const payload = command.payload as Record<string, unknown>
  if (command.method === 'POST') {
    if (row.revision !== '1' || existingIds.has(row.id as string) || row.name !== payload.name || row.parent_id !== payload.parent_id || row.element !== payload.element || row.kind !== payload.kind || row.is_active !== true) refuse()
    return
  }
  if (!before || command.path !== `/api/accounts/${before.id}` || payload.expected_revision !== before.revision ||
      row.id !== before.id || row.revision !== (BigInt(before.revision) + 1n).toString() || row.created_at !== before.createdAt) refuse()
  const expected: Record<string, unknown> = { name: before!.name, parent_id: before!.parentId, element: before!.element, kind: before!.kind, is_active: before!.isActive }
  const fields: Record<string, string> = { rename: 'name', move: 'parent_id', change_element: 'element', change_kind: 'kind', set_active: 'is_active' }
  const field = fields[payload.operation as string]
  if (!field) refuse()
  expected[field!] = payload[field!]
  for (const [key, item] of Object.entries(expected)) if (row[key] !== item) refuse()
}
export function sameSelection(left: { bookId: string; selectionId: string } | null, right: { bookId: string; selectionId: string } | null): boolean {
  return left !== null && right !== null && left.bookId === right.bookId && left.selectionId === right.selectionId
}
