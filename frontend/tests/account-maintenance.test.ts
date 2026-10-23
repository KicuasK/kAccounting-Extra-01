import assert from 'node:assert/strict'
import test from 'node:test'
import type { AccountRecord } from '../src/account-tree.ts'
import { accountFailureRequiresReload, accountPathLabels, changeAccountCommand, createAccountCommand, deleteAccountCommand, eligibleParents, normalizeAccountName, sameSelection, validateAccountCommandResponse } from '../src/account-maintenance-model.ts'
const account = (overrides: Partial<AccountRecord> = {}): AccountRecord => ({ id: '9007199254740993', parentId: null, name: '現金', element: 'asset', kind: 'category', isActive: true, revision: '9007199254740995', createdAt: 'date', updatedAt: 'date', ...overrides })
test('mutation commands preserve IDs and revisions beyond JavaScript safe integer', () => {
  const record = account()
  assert.deepEqual(changeAccountCommand(record, { operation: 'rename', name: ' 新名稱 ' }), { path: '/api/accounts/9007199254740993', method: 'PATCH', payload: { operation: 'rename', name: '新名稱', expected_revision: '9007199254740995' } })
  assert.deepEqual(deleteAccountCommand(record), { path: '/api/accounts/9007199254740993', method: 'DELETE', payload: { expected_revision: '9007199254740995' } })
})
test('root moves send explicit null, activation remains explicit, no journal deletion path exists', () => {
  assert.deepEqual(changeAccountCommand(account(), { operation: 'move', parent_id: null }).payload, { operation: 'move', parent_id: null, expected_revision: '9007199254740995' })
  assert.deepEqual(changeAccountCommand(account(), { operation: 'set_active', is_active: false }).payload, { operation: 'set_active', is_active: false, expected_revision: '9007199254740995' })
  assert.equal(deleteAccountCommand(account()).path.startsWith('/api/accounts/'), true)
})
test('parent candidates exclude self, descendants, posting nodes and other elements', () => {
  const root = account({ id: '1' })
  const child = account({ id: '2', parentId: '1' })
  const grandchild = account({ id: '3', parentId: '2' })
  const records = [grandchild, child, root, account({ id: '4' }), account({ id: '5', kind: 'posting' }), account({ id: '6', element: 'liability' })]
  assert.deepEqual(eligibleParents(records, root).map(item => item.id), ['4'])
})
test('names use Unicode edge whitespace normalization and reject controls or excess scalar count', () => {
  assert.equal(normalizeAccountName('\u3000現金 Cash\u3000'), '現金 Cash')
  assert.equal(normalizeAccountName('𠮷'.repeat(120)).length, 240)
  for (const name of [' ', 'a\nb', 'a\u0000', 'a'.repeat(121)]) assert.throws(() => normalizeAccountName(name))
})
test('create has explicit user-selected classification with no chart or code invention', () => {
  assert.deepEqual(createAccountCommand(' 銀行 ', 'asset', 'posting', null), { path: '/api/accounts', method: 'POST', payload: { name: '銀行', element: 'asset', kind: 'posting', parent_id: null } })
  assert.throws(() => changeAccountCommand(account({ revision: '01' }), { operation: 'rename', name: '現金' }))
  assert.throws(() => createAccountCommand('現金', 'asset', 'posting', '9223372036854775808'))
})
test('selection identity requires both book and generation and never accepts no selection', () => {
  const original = { bookId: 'book', selectionId: 'first' }
  assert.equal(sameSelection(original, { ...original }), true)
  assert.equal(sameSelection(original, { ...original, selectionId: 'second' }), false)
  assert.equal(sameSelection(original, { ...original, bookId: 'other' }), false)
  assert.equal(sameSelection(null, null), false)
})
test('full ancestor labels distinguish same names in different branches without internal IDs', () => {
  const records = [account({ id: '1', name: '流動資產' }), account({ id: '2', name: '銀行', parentId: '1' }), account({ id: '3', name: '非流動資產' }), account({ id: '4', name: '銀行', parentId: '3' })]
  const labels = accountPathLabels(records.reverse())
  assert.equal(labels.get('2'), '資產 / 流動資產 / 銀行')
  assert.equal(labels.get('4'), '資產 / 非流動資產 / 銀行')
  assert.doesNotMatch([...labels.values()].join(), /900719|[0-9]/)
})
test('path helper refuses missing ancestors and cycles instead of ambiguous partial labels', () => {
  assert.throws(() => accountPathLabels([account({ id: '1', parentId: '2' })]))
  assert.throws(() => accountPathLabels([account({ id: '1', parentId: '2' }), account({ id: '2', parentId: '1' })]))
})
test('unknown mutation outcomes and conflicts require explicit reload before any repeat', () => {
  for (const failure of [null, { code: 'request_failed' }, { code: 'internal', status: 500 }, { code: 'gateway', status: 502 }, { code: 'account_revision_conflict', status: 409 }, { code: 'account_not_found', status: 404 }, { code: 'selection_changed', status: 409 }]) assert.equal(accountFailureRequiresReload(failure), true)
  for (const failure of [{ code: 'duplicate_account_name', status: 409 }, { code: 'invalid_account_name', status: 400 }, { code: 'account_rule_violation', status: 422 }, { code: 'duplicate_account_name' }]) assert.equal(accountFailureRequiresReload(failure), false)
})
const wire = (record: AccountRecord): Record<string, unknown> => ({ id: record.id, parent_id: record.parentId, name: record.name, element: record.element, kind: record.kind, is_active: record.isActive, revision: record.revision, created_at: record.createdAt, updated_at: record.updatedAt })
test('PATCH response must match exact target, next revision, requested operation and unchanged fields', () => {
  const before = account(), command = changeAccountCommand(before, { operation: 'rename', name: '新名稱' })
  const valid = { ...wire(before), name: '新名稱', revision: '9007199254740996' }
  assert.doesNotThrow(() => validateAccountCommandResponse(command, valid, before))
  for (const altered of [{ id: '2' }, { revision: before.revision }, { revision: '9007199254740997' }, { name: before.name }, { kind: 'posting' }, { parent_id: '2' }, { element: 'liability' }, { is_active: false }, { created_at: 'changed' }, { updated_at: null }]) assert.throws(() => validateAccountCommandResponse(command, { ...valid, ...altered }, before))
})
test('POST requires a valid complete account matching every submitted field', () => {
  const command = createAccountCommand('新名稱', 'asset', 'posting', null)
  const valid = wire(account({ name: '新名稱', kind: 'posting', revision: '1' }))
  assert.doesNotThrow(() => validateAccountCommandResponse(command, valid))
  for (const bad of [null, {}, { ...valid, id: 1 }, { ...valid, id: '0' }, { ...valid, revision: '01' }, { ...valid, revision: '2' }, { ...valid, name: '其他' }, { ...valid, parent_id: '2' }, { ...valid, element: 'expense' }, { ...valid, kind: 'category' }, { ...valid, is_active: false }]) assert.throws(() => validateAccountCommandResponse(command, bad))
  assert.throws(() => validateAccountCommandResponse(command, valid, undefined, new Set([valid.id as string])))
})
test('DELETE accepts only successful transport 204 null; activation result must acknowledge requested state', () => {
  const before = account(), command = deleteAccountCommand(before)
  assert.doesNotThrow(() => validateAccountCommandResponse(command, null, before))
  for (const bad of [undefined, {}, wire(before), true]) assert.throws(() => validateAccountCommandResponse(command, bad, before))
  const disable = changeAccountCommand(before, { operation: 'set_active', is_active: false })
  assert.throws(() => validateAccountCommandResponse(disable, { ...wire(before), revision: '9007199254740996' }, before))
  assert.doesNotThrow(() => validateAccountCommandResponse(disable, { ...wire(before), revision: '9007199254740996', is_active: false }, before))
})
