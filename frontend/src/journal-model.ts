import { isPositiveI64Decimal, type AccountNode } from './account-tree.ts'
export interface JournalLineInput { accountId: string; accountSearch?: string; debit: string; credit: string; memo: string }
export interface JournalInput { date: string; time?: string; reference: string; explanation: string; lines: JournalLineInput[] }
export interface EntryDocument { entry_date: string; entry_time: string | null; reference: string | null; explanation: string | null; lines: { account_id: string; side: 'debit' | 'credit'; amount_minor: string; memo: string | null }[] }
export interface JournalEntry { id: string; revision: string | null; input: JournalInput; relationship: string; originalId: string | null }
export function amount(value: string): bigint {
  if (!isPositiveI64Decimal(value)) throw new Error('金額須為正整數新臺幣，不接受小數、符號或千分位。')
  return BigInt(value)
}
export function validDate(value: string): boolean {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(value) || value.startsWith('0000')) return false
  const date = new Date(`${value}T00:00:00Z`)
  return Number.isFinite(date.getTime()) && date.toISOString().slice(0, 10) === value
}
export function validTime(value: string): boolean { return /^(?:[01]\d|2[0-3]):[0-5]\d:[0-5]\d$/.test(value) }
export function normalizeTime(value = ''): string | null {
  if (!value) return null
  const normalized = /^\d{2}:\d{2}$/.test(value) ? `${value}:00` : value
  if (!validTime(normalized)) throw new Error('請輸入有效時間（時：分：秒）。')
  return normalized
}
function responseTime(value: unknown): string {
  if (value === undefined || value === null) return ''
  if (typeof value !== 'string' || !validTime(value)) throw new Error('分錄時間格式不符。')
  return value
}
export function journalDocument(input: JournalInput, posting: boolean, activeIds: ReadonlySet<string>): EntryDocument {
  if (!validDate(input.date)) throw new Error('請輸入有效會計日期。')
  let debit = 0n; let credit = 0n
  const lines: EntryDocument['lines'] = []
  input.lines.forEach((line, index) => {
    if (!line.accountId && !line.accountSearch && !line.debit && !line.credit && !line.memo) return
    if (!activeIds.has(line.accountId)) throw new Error(`第 ${index + 1} 列須選擇啟用的過帳科目。`)
    if (Boolean(line.debit) === Boolean(line.credit)) throw new Error(`第 ${index + 1} 列須只填借方或貸方其中一欄。`)
    const side = line.debit ? 'debit' : 'credit'; const value = amount(line.debit || line.credit)
    if (side === 'debit') debit += value; else credit += value
    lines.push({ account_id: line.accountId, side, amount_minor: String(value), memo: line.memo || null })
  })
  if (debit > 9223372036854775807n || credit > 9223372036854775807n) throw new Error('合計金額超過可接受範圍。')
  if (posting && (lines.length < 2 || debit !== credit)) throw new Error('過帳須至少兩列且借貸合計相等。')
  return { entry_date: input.date, entry_time: normalizeTime(input.time), reference: input.reference || null, explanation: input.explanation || null, lines }
}
function record(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('分錄回應格式不符。')
  return value as Record<string, unknown>
}
function text(source: Record<string, unknown>, key: string): string {
  if (typeof source[key] !== 'string') throw new Error('分錄文字格式不符。')
  return source[key]
}
function id(source: Record<string, unknown>, key: string): string {
  const value = text(source, key); if (!isPositiveI64Decimal(value)) throw new Error('分錄識別格式不符。'); return value
}
function optional(source: Record<string, unknown>, key: string): string { return source[key] === null ? '' : text(source, key) }
export function parseJournalEntry(value: unknown, draft: boolean): JournalEntry {
  const source = record(value); const date = text(source, 'entry_date')
  if (!validDate(date) || !Array.isArray(source.lines)) throw new Error('分錄日期或明細格式不符。')
  const seen = new Set<string>()
  const lines = source.lines.map((raw, index) => {
    const line = record(raw); const lineId = id(line, 'id')
    if (seen.has(lineId) || id(line, 'line_no') !== String(index + 1)) throw new Error('分錄明細順序不符。')
    seen.add(lineId)
    const value = id(line, 'amount_minor'); const side = text(line, 'side')
    if (side !== 'debit' && side !== 'credit') throw new Error('分錄借貸方向不符。')
    return { accountId: id(line, 'account_id'), debit: side === 'debit' ? value : '', credit: side === 'credit' ? value : '', memo: optional(line, 'memo') }
  })
  if (!draft && !['ordinary', 'correction', 'reversal'].includes(text(source, 'relationship'))) throw new Error('分錄關係格式不符。')
  if (!draft) {
    const debit = lines.reduce((sum, line) => sum + (line.debit ? amount(line.debit) : 0n), 0n)
    const credit = lines.reduce((sum, line) => sum + (line.credit ? amount(line.credit) : 0n), 0n)
    if (lines.length < 2 || debit !== credit || debit > 9223372036854775807n) throw new Error('已過帳分錄未通過借貸核對。')
    if ((source.relationship === 'ordinary') !== (source.original_entry_id === null)) throw new Error('分錄連結格式不符。')
    if (source.original_entry_id === source.id || (source.relationship === 'correction' && !optional(source, 'explanation').trim())) throw new Error('更正或沖銷連結格式不符。')
  }
  return { id: id(source, 'id'), revision: draft ? id(source, 'revision') : null, input: { date, time: responseTime(source.entry_time), reference: optional(source, 'reference'), explanation: optional(source, 'explanation'), lines }, relationship: draft ? 'ordinary' : text(source, 'relationship'), originalId: draft || source.original_entry_id === null ? null : id(source, 'original_entry_id') }
}
export function entryDocument(entry: JournalEntry): EntryDocument {
  return { entry_date: entry.input.date, entry_time: normalizeTime(entry.input.time), reference: entry.input.reference || null, explanation: entry.input.explanation || null, lines: entry.input.lines.map(line => ({ account_id: line.accountId, side: line.debit ? 'debit' : 'credit', amount_minor: line.debit || line.credit, memo: line.memo || null })) }
}
export function verifyJournalResult(entry: JournalEntry, expected: EntryDocument, operation: 'create' | 'replace' | 'post' | 'correction' | 'reversal', target?: JournalEntry): void {
  if (JSON.stringify(entryDocument(entry)) !== JSON.stringify(expected)) throw new Error('分錄回應與已確認內容不符；請核對寫入結果。')
  if (operation === 'create' || operation === 'replace') {
    if (entry.revision === null || (operation === 'create' && entry.revision !== '1')) throw new Error('草稿回應版本不符。')
    if (operation === 'replace' && (!target?.revision || entry.id !== target.id || BigInt(entry.revision) !== BigInt(target.revision) + 1n)) throw new Error('草稿回應識別或版本不符。')
  } else {
    if (entry.revision !== null || !target) throw new Error('過帳回應狀態不符。')
    if (operation === 'post' ? entry.id !== target.id || entry.relationship !== 'ordinary' || entry.originalId !== null : entry.id === target.id || entry.relationship !== operation || entry.originalId !== target.id) throw new Error('已過帳回應識別或連結不符。')
  }
}
export function reversalDocument(original: JournalEntry, date: string, time = ''): EntryDocument {
  return { entry_date: date, entry_time: normalizeTime(time), reference: null, explanation: null, lines: entryDocument(original).lines.map(line => ({ account_id: line.account_id, side: line.side === 'debit' ? 'credit' : 'debit', amount_minor: line.amount_minor, memo: null })) }
}
export function parseDraftList(value: unknown): JournalEntry[] {
  if (!Array.isArray(value) || value.length > 5000) throw new Error('草稿清單格式或筆數不符。')
  const seen = new Set<string>()
  return value.map(raw => { const entry = parseJournalEntry(raw, true); if (seen.has(entry.id)) throw new Error('草稿清單識別重複。'); seen.add(entry.id); return entry })
}
export function draftSaveCommand(entry: JournalEntry | null, document: EntryDocument): { path: string; method: string; payload: unknown } {
  if (entry && entry.revision === null) throw new Error('已過帳分錄不可覆寫。')
  return entry ? { path: `/api/journal/drafts/${entry.id}`, method: 'PUT', payload: { expected_revision: entry.revision, entry: document } } : { path: '/api/journal/drafts', method: 'POST', payload: document }
}
export function postingAccounts(nodes: AccountNode[]): { id: string; label: string }[] {
  const result: { id: string; label: string }[] = []
  function walk(items: AccountNode[], parents: string[]): void { items.forEach(node => { const names = [...parents, node.account.name]; if (node.account.isActive && node.account.kind === 'posting') result.push({ id: node.account.id, label: names.join(' › ') }); walk(node.children, names) }) }
  walk(nodes, []); return result
}
