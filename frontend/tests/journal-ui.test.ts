import { test } from 'node:test'
import assert from 'node:assert/strict'
import { mountJournalUI } from '../src/journal-ui.ts'
import { ApiError } from '../src/session-client.ts'
import { mountAttachmentsUI } from '../src/attachments-ui.ts'

class Node {
  tag: string; value = ''; disabled = false; className = ''; type = ''; id = ''; parent: Node | null = null
  private ownText = ''; children: Node[] = []; attrs = new Map<string, string>(); listeners = new Map<string, (() => void)[]>()
  constructor(tag: string) { this.tag = tag }
  set textContent(value: string) { this.ownText = value; this.children = [] }
  get textContent(): string { return this.ownText + this.children.map(child => child.textContent).join('') }
  append(...nodes: (Node | string)[]): void { nodes.forEach(node => { const child = typeof node === 'string' ? new Node('text') : node; if (typeof node === 'string') child.textContent = node; child.remove(); child.parent = this; this.children.push(child) }) }
  replaceChildren(...nodes: Node[]): void { this.children.forEach(node => { node.parent = null }); this.children = []; this.ownText = ''; this.append(...nodes) }
  remove(): void { if (this.parent) this.parent.children = this.parent.children.filter(node => node !== this); this.parent = null }
  setAttribute(key: string, value: string): void { this.attrs.set(key, value) }
  addEventListener(key: string, listener: () => void): void { this.listeners.set(key, [...this.listeners.get(key) ?? [], listener]) }
  querySelector(selector: string): Node | null { return this.all().find(node => selector.startsWith('.') ? node.className.split(/\s+/).includes(selector.slice(1)) : node.tag === selector) ?? null }
  all(): Node[] { return this.children.flatMap(child => [child, ...child.all()]) }
  fire(event: string): void { if (!this.disabled) this.listeners.get(event)?.forEach(listener => listener()) }
}
function install(): Node {
  Object.defineProperty(globalThis, 'document', { configurable: true, value: { createElement: (tag: string) => new Node(tag) } })
  Object.defineProperty(globalThis, 'window', { configurable: true, value: { confirm: () => true } })
  return new Node('host')
}
async function settle(): Promise<void> { for (let i = 0; i < 50; i++) await Promise.resolve() }
const account = (id: string, name: string) => ({ account: { id, parent_id: null, name, element: 'asset', kind: 'posting', is_active: true, revision: '1', created_at: 'now', updated_at: 'now' }, children: [] })
const tree = [account('1', '現金'), account('2', '銀行')]
function wire(id = '8', draft = true): Record<string, unknown> { return { id, ...(draft ? { revision: '1' } : { relationship: 'ordinary', original_entry_id: null }), entry_date: '2026-10-23', reference: '', explanation: '', lines: [{ id: '1', line_no: '1', account_id: '1', side: 'debit', amount_minor: '10', memo: null }, { id: '2', line_no: '2', account_id: '2', side: 'credit', amount_minor: '10', memo: null }] } }
function button(host: Node, text: string): Node { const node = host.all().find(node => node.tag === 'button' && node.textContent === text); assert.ok(node, `button ${text}`); return node }
function field(host: Node, label: string): Node { const node = host.all().find(node => node.tag === 'label' && node.textContent === label)?.querySelector('input'); assert.ok(node, label); return node }
function edit(node: Node, value: string): void { node.value = value; node.fire('input') }
function setup(handler: (path: string, method: string, payload: unknown) => Promise<unknown>, upload?: (path: string, signal: AbortSignal, file: File) => Promise<unknown>) {
  const host = install(); const calls: { path: string; method: string; payload: unknown }[] = []
  const ui = mountJournalUI(host as unknown as HTMLElement, { upload, getSelection: () => ({ bookId: 'test', selectionId: 'a'.repeat(32) }), async request(path, _signal, method = 'GET', payload) { calls.push({ path, method, payload }); if (path === '/api/accounts/tree') return tree; if (/\/entries\/\d+\/attachments$/.test(path)) return []; return handler(path, method, payload) }, onChanged() {}, onSelectionChanged() {}, createAccountName(name) { const node = new Node('span'); node.textContent = name; return node as unknown as HTMLElement } })
  return { host, calls, ui }
}
function fill(host: Node): void {
  edit(field(host, '會計日期'), '2026-10-23')
  const accounts = host.all().filter(node => node.attrs.get('aria-label')?.includes('列科目名稱'))
  edit(accounts[0]!, '現金'); edit(accounts[1]!, '銀行'); edit(field(host, '第 1 列借方'), '10'); edit(field(host, '第 2 列貸方'), '10')
}
test('queued posted navigation waits for initial loading and opens immutable matching ID', async () => {
  const { host, calls, ui } = setup(async path => path === '/api/journal/drafts' ? [] : wire('8', false))
  assert.equal(ui.hasUnsavedChanges(), true); ui.openPosted('8'); await settle()
  assert.ok(calls.some(call => call.path === '/api/journal/posted/8')); assert.ok(host.textContent.includes('已過帳分錄（不可修改）')); assert.equal(field(host, '會計日期').disabled, true); assert.equal(ui.hasUnsavedChanges(), false); ui.dispose()
})
test('payment draft navigation waits for loading and never posts automatically', async () => {
  const { host, calls, ui } = setup(async path => path === '/api/journal/drafts' ? [] : wire('18'))
  ui.openDraft('18'); await settle()
  assert.ok(calls.some(call => call.path === '/api/journal/drafts/18'))
  assert.equal(calls.some(call => call.method !== 'GET'), false)
  assert.equal(field(host, '會計日期').disabled, false)
  assert.match(host.textContent, /核對內容/)
  assert.equal(ui.hasUnsavedChanges(), false); ui.dispose()
})
test('payment draft navigation rejects mismatched response identity', async () => {
  const { host, ui } = setup(async path => path === '/api/journal/drafts' ? [] : wire('19'))
  await settle(); ui.openDraft('18'); await settle()
  assert.match(host.textContent, /草稿回應識別不符/); ui.dispose()
})
test('422 keeps inputs and permits an explicit corrected save without leaking server code', async () => {
  let attempts = 0
  const { host, calls, ui } = setup(async (path, method) => { if (method === 'GET') return []; attempts++; if (attempts === 1) { const error = new ApiError('<untrusted>'); Object.assign(error, { status: 422 }); throw error }; return wire() })
  await settle(); fill(host); button(host, '儲存草稿').fire('click'); await settle(); assert.equal(field(host, '第 1 列借方').value, '10'); assert.ok(!host.textContent.includes('<untrusted>'))
  edit(field(host, '參考編號'), ''); button(host, '儲存草稿').fire('click'); await settle(); assert.equal(calls.filter(call => call.method === 'POST').length, 2); assert.equal(ui.hasUnsavedChanges(), false); ui.dispose()
})
test('unknown save outcome preserves inputs and blocks any repeated mutation', async () => {
  const { host, calls, ui } = setup(async (_path, method) => { if (method === 'GET') return []; throw new TypeError('offline') })
  await settle(); fill(host); button(host, '儲存草稿').fire('click'); await settle(); button(host, '儲存草稿').fire('click'); await settle()
  assert.equal(calls.filter(call => call.method === 'POST').length, 1); assert.equal(field(host, '第 1 列借方').value, '10'); assert.equal(ui.hasUnsavedChanges(), true); ui.dispose()
})
test('editing a field invalidates confirmation and one pending confirmation cannot post twice', async () => {
  let resolvePost: ((value: unknown) => void) | undefined
  const { host, calls, ui } = setup(async (path, method) => { if (path === '/api/journal/drafts') return [wire()]; if (method === 'GET') return wire(); return new Promise(resolve => { resolvePost = resolve }) })
  await settle(); button(host, '2026-10-23　未填參考編號　').fire('click'); await settle(); button(host, '過帳已儲存草稿').fire('click'); const oldConfirm = button(host, '確認'); edit(field(host, '參考編號'), 'edited'); assert.ok(!host.all().some(node => node.tag === 'button' && node.textContent === '確認'))
  // Reload requires explicit discard; original saved document can now be posted.
  button(host, '2026-10-23　未填參考編號　').fire('click'); await settle(); button(host, '過帳已儲存草稿').fire('click'); const confirm = button(host, '確認'); confirm.fire('click'); confirm.fire('click'); await settle(); assert.equal(calls.filter(call => call.path.endsWith('/post')).length, 1)
  assert.ok(oldConfirm); resolvePost!(wire('8', false)); await settle(); assert.ok(host.textContent.includes('已過帳分錄（不可修改）')); ui.dispose()
})
test('unknown reversal result leaves unsaved guard set and prevents blind second reversal', async () => {
  const { host, calls, ui } = setup(async (path, method) => { if (path === '/api/journal/drafts') return []; if (method === 'GET') return wire('8', false); throw new Error('offline') })
  await settle(); ui.openPosted('8'); await settle(); button(host, '沖銷分錄').fire('click'); button(host, '確認').fire('click'); await settle(); assert.equal(ui.hasUnsavedChanges(), true)
  button(host, '沖銷分錄').fire('click'); assert.ok(!host.all().some(node => node.tag === 'button' && node.textContent === '確認')); assert.equal(calls.filter(call => call.path.endsWith('/reversal')).length, 1); ui.dispose()
})
test('wrong-ID read and wrong-content write responses never replace edited state', async () => {
  const { host, ui } = setup(async (path, method) => { if (path === '/api/journal/drafts') return method === 'GET' ? [wire()] : { ...wire(), reference: 'wrong' }; return wire('9') })
  await settle(); button(host, '2026-10-23　未填參考編號　').fire('click'); await settle(); assert.ok(host.textContent.includes('草稿回應識別不符')); fill(host); button(host, '儲存草稿').fire('click'); await settle(); assert.equal(field(host, '參考編號').value, ''); assert.equal(ui.hasUnsavedChanges(), true); ui.dispose()
})
const attachment = { id: 'a'.repeat(32), entry_id: '8', display_name: '收據.pdf', byte_length: '10', sha256: 'b'.repeat(64), created_at: 'now' }
test('attachments upload stays original File and posts only after confirmation; draft reloads afterward', async () => {
  const host = install(); let uploaded: File | null = null; let reloads = 0; let published = false; let path = ''
  const ui = mountAttachmentsUI(host as unknown as HTMLElement, { request: async () => published ? [attachment] : [], upload: async (target, _signal, file) => { path = target; uploaded = file; published = true; return attachment }, onEntryChanged: async () => { reloads++ }, onBusy() {}, onSelectionChanged() {} })
  ui.setEntry({ id: '8', revision: '9007199254740993' }, false); await settle()
  const picker = host.all().find(node => node.type === 'file')!; const file = { name: '收據.pdf', size: 10 } as File; Object.assign(picker, { files: { item: () => file } }); picker.fire('change'); assert.equal(ui.hasUnsavedChanges(), true)
  button(host, '新增附件').fire('click'); assert.equal(uploaded, null); button(host, '確認新增附件').fire('click'); await settle()
  assert.equal(uploaded, file); assert.match(path, /expected_revision=9007199254740993$/); assert.match(path, /display_name=%/); assert.equal(reloads, 1); assert.ok(host.textContent.includes('收據.pdf')); assert.equal(ui.hasUnsavedChanges(), false); ui.dispose()
})
test('posted attachments never expose detach and download only hands metadata to native transport', async () => {
  const host = install(); let downloadPath = ''; let verified = false
  const ui = mountAttachmentsUI(host as unknown as HTMLElement, { request: async (path, _signal, method) => { if (method === 'POST') { verified = true; return { id: attachment.id, verified: true } }; return [attachment] }, download: async path => { downloadPath = path }, onEntryChanged: async () => {}, onBusy() {}, onSelectionChanged() {} })
  ui.setEntry({ id: '8', revision: null }, false); await settle(); assert.ok(!host.all().some(node => node.textContent === '移除附件'))
  button(host, '驗證附件').fire('click'); await settle(); assert.equal(verified, true); assert.ok(host.textContent.includes('完整性已驗證'))
  button(host, '下載附件').fire('click'); await settle(); assert.equal(downloadPath, `/api/attachments/${attachment.id}/content`); assert.ok(host.textContent.includes('請在 Safari 下載項目確認完成')); ui.dispose()
})
test('unknown upload result blocks another upload until explicit reconciliation and never clears selected File', async () => {
  const host = install(); let uploads = 0
  const ui = mountAttachmentsUI(host as unknown as HTMLElement, { request: async () => [], upload: async () => { uploads++; throw new Error('offline') }, onEntryChanged: async () => {}, onBusy() {}, onSelectionChanged() {} })
  ui.setEntry({ id: '8', revision: '1' }, false); await settle(); const picker = host.all().find(node => node.type === 'file')!; Object.assign(picker, { files: { item: () => ({ name: 'a.pdf', size: 10 }) } }); picker.fire('change'); button(host, '新增附件').fire('click'); button(host, '確認新增附件').fire('click'); await settle(); assert.equal(uploads, 1); assert.equal(ui.hasUnsavedChanges(), true); assert.equal(button(host, '新增附件').disabled, true)
  button(host, '重新載入附件').fire('click'); await settle(); assert.equal(ui.hasUnsavedChanges(), true); assert.equal(button(host, '新增附件').disabled, false); ui.dispose()
})
test('draft attachment detach confirms exact current revision and refreshes entry before list', async () => {
  const host = install(); const operations: string[] = []; let payload: unknown; let detached = false
  const ui = mountAttachmentsUI(host as unknown as HTMLElement, { request: async (_path, _signal, method = 'GET', body) => { operations.push(method); if (method === 'DELETE') { payload = body; detached = true; return null }; return detached ? [] : [attachment] }, onEntryChanged: async () => { operations.push('revision-refresh') }, onBusy() {}, onSelectionChanged() {} })
  ui.setEntry({ id: '8', revision: '9007199254740993' }, false); await settle(); button(host, '移除附件').fire('click'); assert.equal(payload, undefined); button(host, '確認移除附件').fire('click'); await settle()
  assert.deepEqual(payload, { expected_revision: '9007199254740993' }); assert.deepEqual(operations, ['GET', 'DELETE', 'revision-refresh', 'GET']); assert.ok(host.textContent.includes('0 份附件')); ui.dispose()
})
test('editing saved journal fields immediately disables attachment mutation before saving', async () => {
  const { host, ui } = setup(async path => path === '/api/journal/drafts' ? [wire()] : wire(), async () => attachment)
  await settle(); button(host, '2026-10-23　未填參考編號　').fire('click'); await settle()
  const picker = host.all().find(node => node.type === 'file')!
  Object.assign(picker, { files: { item: () => ({ name: '收據.pdf', size: 10 }) } }); picker.fire('change')
  assert.equal(button(host, '新增附件').disabled, false)
  edit(field(host, '參考編號'), 'keep unsaved'); assert.equal(button(host, '新增附件').disabled, true); assert.equal(ui.hasUnsavedChanges(), true); ui.dispose()
})

test('general journal keeps date and narrative in the table without PR and shows posted amounts as text', async () => {
  const result = wire('8', false)
  result.explanation = '購置用品'
  const { host, ui } = setup(async path => path === '/api/journal/drafts' ? [] : result)
  ui.openPosted('8'); await settle()
  const table = host.all().find(node => node.tag === 'table' && node.all().some(child => child.tag === 'th' && child.textContent === '科目／摘要'))!
  assert.ok(table)
  assert.deepEqual(table.all().filter(node => node.tag === 'th').map(node => node.textContent), ['日期', '科目／摘要', '借方 TWD', '貸方 TWD', ''])
  assert.equal(table.all().filter(node => node.tag === 'input').length, 0)
  assert.ok(table.textContent.includes('2026-10-23'))
  assert.ok(table.textContent.includes('（購置用品）'))
  assert.equal(table.all().filter(node => node.className === 'journal-credit-account').length, 1)
  assert.equal(table.all().filter(node => node.className === 'amount-column' && node.textContent === '10').length, 2)
  ui.dispose()
})


test('account annotation controls preserve their save binding', async () => {
  const { host, calls, ui } = setup(async () => [])
  await settle()
  const notes = host.all().find(node => node.tag === 'details' && node.className.startsWith('journal-note'))!
  assert.ok(notes)
  assert.equal(notes.attrs.has('open'), false)
  const memo = field(host, '第 1 列摘要')
  assert.ok(notes.all().includes(memo))
  const table = host.all().find(node => node.tag === 'table')!
  assert.ok(table.all().includes(memo))
  fill(host)
  assert.equal(field(host, '第 1 列貸方').value, '')
  assert.equal(field(host, '第 2 列借方').value, '')
  edit(memo, '保留這筆分行備註')
  button(host, '儲存草稿').fire('click'); await settle()
  const saved = calls.find(call => call.method === 'POST')!.payload as { lines: { memo: string }[] }
  assert.equal(saved.lines[0]!.memo, '保留這筆分行備註')
  ui.dispose()
})

test('posted account annotations remain readable without editing', async () => {
  const entry = wire('8', false)
  ;(entry.lines as { memo: string | null }[])[0]!.memo = '既有備註'
  const { host, ui } = setup(async path => path === '/api/journal/drafts' ? [] : entry)
  ui.openPosted('8'); await settle()
  const notes = host.all().find(node => node.tag === 'details' && node.className.startsWith('journal-note'))!
  assert.ok(notes.className.includes('has-note'))
  assert.ok(notes.textContent.includes('既有備註'))
  assert.equal(notes.all().filter(node => node.tag === 'input').length, 0)
  ui.dispose()
})

test('attachment chooser mirrors native disabled state while preserving selected File across blocked toggles', async () => {
  const host = install(); let uploaded: File | null = null; let published = false
  const ui = mountAttachmentsUI(host as unknown as HTMLElement, { request: async () => published ? [attachment] : [], upload: async (_path, _signal, file) => { uploaded = file; published = true; return attachment }, onEntryChanged: async () => {}, onBusy() {}, onSelectionChanged() {} })
  const entry = { id: '8', revision: '1' }
  const checkDisabled = (expected: boolean): void => {
    assert.equal(host.all().find(node => node.type === 'file')!.disabled, expected)
    assert.equal(button(host, '選擇檔案').disabled, expected)
  }
  ui.setEntry(entry, false); await settle(); checkDisabled(false)
  const file = { name: '收據.pdf', size: 10 } as File
  const picker = host.all().find(node => node.type === 'file')!
  Object.assign(picker, { files: { item: () => file } }); picker.fire('change')
  ui.setEntry(entry, true); checkDisabled(true)
  assert.equal(ui.hasUnsavedChanges(), true)
  assert.ok(host.textContent.includes(file.name))
  ui.setEntry(entry, false); checkDisabled(false)
  button(host, '新增附件').fire('click'); assert.equal(uploaded, null)
  button(host, '確認新增附件').fire('click'); await settle()
  assert.equal(uploaded, file)
  assert.equal(ui.hasUnsavedChanges(), false)
  ui.dispose()
})

test('attachment status remains announced beside heading and unavailable upload disables both choosers', async () => {
  const host = install()
  const ui = mountAttachmentsUI(host as unknown as HTMLElement, { request: async () => [], onEntryChanged: async () => {}, onBusy() {}, onSelectionChanged() {} })
  ui.setEntry({ id: '8', revision: '1' }, false); await settle()
  const heading = host.all().find(node => node.tag === 'h3' && node.textContent === '分錄附件')!
  const status = host.all().find(node => node.attrs.get('role') === 'status')!
  assert.ok(heading); assert.ok(status)
  assert.equal(status.parent, heading.parent)
  assert.equal(status.attrs.get('aria-live'), 'polite')
  assert.match(status.textContent, /附件清單已讀取/)
  assert.equal(host.all().find(node => node.type === 'file')!.disabled, true)
  assert.equal(button(host, '選擇檔案').disabled, true)
  ui.dispose()
})

test('updating journal totals preserves the single add-row control and its action', async () => {
  const { host, ui } = setup(async () => [])
  await settle()
  const summary = () => host.all().find(node => node.tag === 'footer' && node.className.includes('journal-summary'))!
  const assertAddRow = (): void => {
    const controls = host.all().filter(node => node.tag === 'button' && node.textContent === '新增列')
    assert.equal(controls.length, 1)
    assert.ok(summary().all().includes(controls[0]!))
  }
  assertAddRow()
  edit(field(host, '第 1 列借方'), '25')
  assert.match(summary().textContent, /借方 25/)
  assertAddRow()
  edit(field(host, '第 2 列貸方'), '25')
  assertAddRow()
  button(host, '新增列').fire('click')
  assert.equal(host.all().filter(node => node.attrs.get('aria-label')?.includes('列科目名稱')).length, 3)
  assert.equal(field(host, '第 1 列借方').value, '25')
  assert.equal(field(host, '第 2 列貸方').value, '25')
  assertAddRow()
  ui.dispose()
})

test('direct draft trash control requires both confirmations and either cancel prevents deletion', async () => {
  const { host, calls, ui } = setup(async (_path, method) => method === 'DELETE' ? null : _path === '/api/journal/drafts' ? [wire()] : wire())
  await settle(); ui.openDraft('8'); await settle()
  const remove = button(host, '刪除草稿')
  assert.equal(remove.attrs.get('aria-label'), '刪除草稿')
  assert.ok(!host.all().some(node => node.tag === 'details' && node.all().includes(remove)))
  const deleteCount = (): number => calls.filter(call => call.method === 'DELETE').length
  remove.fire('click'); assert.equal(deleteCount(), 0)
  button(host, '取消').fire('click'); assert.equal(deleteCount(), 0)
  button(host, '刪除草稿').fire('click'); button(host, '繼續刪除').fire('click')
  assert.equal(deleteCount(), 0)
  button(host, '取消').fire('click'); assert.equal(deleteCount(), 0)
  button(host, '刪除草稿').fire('click'); button(host, '繼續刪除').fire('click')
  assert.equal(deleteCount(), 0)
  const finalConfirmation = button(host, '確認刪除')
  finalConfirmation.fire('click'); finalConfirmation.fire('click'); await settle()
  const deletes = calls.filter(call => call.method === 'DELETE')
  assert.equal(deletes.length, 1)
  assert.equal(deletes[0]!.path, '/api/journal/drafts/8')
  assert.deepEqual(deletes[0]!.payload, { expected_revision: '1' })
  ui.dispose()
})

test('optional journal time updates the date cell and saves exact seconds', async () => {
  const { host, calls, ui } = setup(async (_path, method, payload) => method === 'GET' ? [] : { ...wire(), entry_time: (payload as { entry_time: string }).entry_time })
  await settle(); fill(host)
  const time = field(host, '時間（選填）')
  assert.equal(time.type, 'text')
  assert.equal((time as unknown as { placeholder: string }).placeholder, 'HH:mm:ss')
  assert.equal(time.value, '')
  edit(time, '09:08:07')
  assert.equal(host.querySelector('.journal-entry-date')!.textContent, '2026-10-23\n09:08:07')
  assert.equal(ui.hasUnsavedChanges(), true)
  button(host, '儲存草稿').fire('click'); await settle()
  const saved = calls.find(call => call.method === 'POST')!.payload as { entry_time: string }
  assert.equal(saved.entry_time, '09:08:07')
  assert.equal(field(host, '時間（選填）').value, '09:08:07')
  assert.equal(ui.hasUnsavedChanges(), false)
  ui.dispose()
})

test('clearing optional journal time removes it from display and submits null', async () => {
  const { host, calls, ui } = setup(async (_path, method) => method === 'GET' ? [] : { ...wire(), entry_time: null })
  await settle(); fill(host)
  edit(field(host, '時間（選填）'), '13:45')
  assert.equal(host.querySelector('.journal-entry-date')!.textContent, '2026-10-23\n13:45:00')
  edit(field(host, '時間（選填）'), '')
  assert.equal(host.querySelector('.journal-entry-date')!.textContent, '2026-10-23')
  button(host, '儲存草稿').fire('click'); await settle()
  assert.equal((calls.find(call => call.method === 'POST')!.payload as { entry_time: unknown }).entry_time, null)
  assert.equal(ui.hasUnsavedChanges(), false)
  ui.dispose()
})

test('posted journal time remains visible and cannot be edited', async () => {
  const { host, calls, ui } = setup(async path => path === '/api/journal/drafts' ? [] : { ...wire('8', false), entry_time: '23:59:58' })
  ui.openPosted('8'); await settle()
  const time = field(host, '時間（選填）')
  assert.equal(time.disabled, true)
  assert.equal(time.value, '23:59:58')
  assert.equal(host.querySelector('.journal-entry-date')!.textContent, '2026-10-23\n23:59:58')
  assert.equal(calls.some(call => call.method !== 'GET'), false)
  assert.equal(ui.hasUnsavedChanges(), false)
  ui.dispose()
})


test('24-hour time permits partial editing and rejects invalid times before a write', async () => {
  const { host, calls, ui } = setup(async () => [])
  await settle(); fill(host)
  const time = field(host, '時間（選填）')
  for (const value of ['1', '14:', '14:3', '14:35:', '14:35:2']) assert.doesNotThrow(() => edit(time, value))
  button(host, '新增列').fire('click')
  assert.equal(field(host, '時間（選填）').value, '14:35:2')
  edit(field(host, '時間（選填）'), '24:00:00'); button(host, '儲存草稿').fire('click'); await settle()
  assert.equal(calls.some(call => call.method !== 'GET'), false)
  assert.equal(field(host, '時間（選填）').value, '24:00:00')
  ui.dispose()
})


test('invalid 24-hour reversal time cannot reach confirmation or a write', async () => {
  const { host, calls, ui } = setup(async path => path === '/api/journal/drafts' ? [] : wire('8', false))
  ui.openPosted('8'); await settle()
  const time = host.all().find(node => node.attrs.get('aria-label') === '沖銷時間（選填）')!
  assert.equal(time.type, 'text')
  edit(time, '24:00:00'); button(host, '沖銷分錄').fire('click')
  assert.ok(!host.all().some(node => node.tag === 'button' && node.textContent === '確認'))
  assert.equal(calls.some(call => call.method !== 'GET'), false)
  ui.dispose()
})
