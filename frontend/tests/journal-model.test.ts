import { test } from 'node:test'
import assert from 'node:assert/strict'
import { amount, draftSaveCommand, entryDocument, journalDocument, parseDraftList, parseJournalEntry, postingAccounts, reversalDocument, validDate, verifyJournalResult, type JournalInput } from '../src/journal-model.ts'

const input = (): JournalInput => ({ date: '2026-10-23', reference: '', explanation: '', lines: [{ accountId: '1', debit: '9007199254740993', credit: '', memo: '' }, { accountId: '2', debit: '', credit: '9007199254740993', memo: '' }] })
const active = new Set(['1', '2'])
test('money remains exact above the JavaScript integer boundary', () => {
  assert.equal(amount('9007199254740993'), 9007199254740993n)
  const document = journalDocument(input(), true, active)
  assert.equal(document.lines[0]?.amount_minor, '9007199254740993')
  assert.equal(document.lines[1]?.side, 'credit')
  assert.equal(document.reference, null)
})
test('whole positive TWD rejects floats signs separators exponent zero leading zeros and overflow', () => {
  for (const value of ['0', '-1', '+1', '1.00', '1,000', '1e3', '01', ' 1', '9223372036854775808']) assert.throws(() => amount(value))
})
test('posting requires two lines equal totals and active accounts', () => {
  const document = input(); document.lines[1]!.credit = '2'; assert.throws(() => journalDocument(document, true, active))
  assert.equal(journalDocument(document, false, active).lines.length, 2)
  assert.throws(() => journalDocument(input(), true, new Set(['1'])))
  const one = input(); one.lines.pop(); assert.throws(() => journalDocument(one, true, active))
})
test('both sides on a line rejected and incomplete nonblank line never silently omitted', () => {
  const both = input(); both.lines[0]!.credit = '3'; assert.throws(() => journalDocument(both, false, active))
  const incomplete = input(); incomplete.lines.push({ accountId: '', debit: '', credit: '', memo: 'keep me' }); assert.throws(() => journalDocument(incomplete, false, active))
  incomplete.lines[2]!.memo = ''; assert.equal(journalDocument(incomplete, false, active).lines.length, 2)
  incomplete.lines[2]!.accountSearch = '尚未選擇的科目'; assert.throws(() => journalDocument(incomplete, false, active))
})
test('checked totals reject signed 64-bit overflow even for unbalanced drafts', () => {
  const document = input(); document.lines[0]!.debit = '9223372036854775807'; document.lines.push({ accountId: '1', debit: '1', credit: '', memo: '' }); assert.throws(() => journalDocument(document, false, active))
})
test('calendar dates validate including leap years', () => {
  assert.equal(validDate('2024-02-29'), true); assert.equal(validDate('2026-02-29'), false); assert.equal(validDate('0000-01-01'), false); assert.equal(validDate('2026-13-01'), false)
})
function wire(revision: unknown = '9007199254740993'): unknown { return { id: '8', revision, entry_date: '2026-10-23', reference: null, explanation: 'test', lines: [{ id: '6', line_no: '1', account_id: '1', side: 'debit', amount_minor: '9007199254740993', memo: null }] } }
test('draft replacement pins the exact revision and never mutates the input', () => {
  const entry = parseJournalEntry(wire(), true); const document = journalDocument(input(), false, active)
  assert.deepEqual(draftSaveCommand(entry, document), { path: '/api/journal/drafts/8', method: 'PUT', payload: { expected_revision: '9007199254740993', entry: document } })
  assert.equal(entry.input.lines[0]?.debit, '9007199254740993')
  assert.equal(draftSaveCommand(null, document).method, 'POST')
})
test('posted entries cannot be passed to the draft overwrite command', () => {
  assert.throws(() => draftSaveCommand({ id: '1', revision: null, input: input(), relationship: 'ordinary', originalId: null }, journalDocument(input(), true, active)))
})
test('wire rejects numeric revisions and unsorted or duplicate line metadata', () => {
  assert.throws(() => parseJournalEntry(wire(1), true))
  const source = wire() as { lines: Record<string, unknown>[] }; source.lines[0]!.line_no = '2'; assert.throws(() => parseJournalEntry(source, true))
})
test('failed validation preserves all original inputs including unrecognized lookup text', () => {
  const document = input(); document.lines[0]!.accountSearch = '尚未選取'; document.lines[0]!.accountId = ''; const before = structuredClone(document)
  assert.throws(() => journalDocument(document, true, active)); assert.deepEqual(document, before)
})
test('posted response requires balanced exact amounts and permanent relationship consistency', () => {
  const entry = { id: '8', entry_date: '2026-10-23', reference: null, explanation: null, relationship: 'ordinary', original_entry_id: null, lines: [{ id: '6', line_no: '1', account_id: '1', side: 'debit', amount_minor: '10', memo: null }, { id: '7', line_no: '2', account_id: '2', side: 'credit', amount_minor: '10', memo: null }] }
  assert.equal(parseJournalEntry(entry, false).revision, null)
  assert.throws(() => parseJournalEntry({ ...entry, original_entry_id: '3' }, false))
  assert.throws(() => parseJournalEntry({ ...entry, lines: entry.lines.slice(0, 1) }, false))
})
test('account lookup contains names and ancestor path, excludes inactive and categories', () => {
  const account = { id: '1', parentId: null, name: '資產', element: 'asset' as const, kind: 'category' as const, isActive: true, revision: '1', createdAt: '', updatedAt: '' }
  const result = postingAccounts([{ account, children: [{ account: { ...account, id: '2', name: '銀行', parentId: '1', kind: 'posting' }, children: [] }, { account: { ...account, id: '3', name: '舊戶', kind: 'posting', isActive: false }, children: [] }] }])
  assert.deepEqual(result, [{ id: '2', label: '資產 › 銀行' }])
})
test('save responses must match submitted document target and exact incremented revision', () => {
  const target = parseJournalEntry(wire(), true); const expected = entryDocument(target)
  const saved = { ...target, revision: '9007199254740994' }; assert.doesNotThrow(() => verifyJournalResult(saved, expected, 'replace', target))
  assert.throws(() => verifyJournalResult(target, expected, 'replace', target))
  assert.throws(() => verifyJournalResult({ ...saved, id: '9' }, expected, 'replace', target))
  assert.throws(() => verifyJournalResult({ ...saved, input: { ...saved.input, reference: 'unexpected' } }, expected, 'replace', target))
  assert.doesNotThrow(() => verifyJournalResult({ ...target, revision: '1' }, expected, 'create'))
  assert.throws(() => verifyJournalResult(saved, expected, 'create'))
})
test('post correction and reversal results require exact identity permanent link and document', () => {
  const target = parseJournalEntry(wire(), true); const document = entryDocument(target); const posted = { ...target, revision: null }
  assert.doesNotThrow(() => verifyJournalResult(posted, document, 'post', target))
  assert.throws(() => verifyJournalResult({ ...posted, id: '9' }, document, 'post', target))
  const correction = { ...posted, id: '9', relationship: 'correction', originalId: target.id }
  assert.doesNotThrow(() => verifyJournalResult(correction, document, 'correction', target))
  assert.throws(() => verifyJournalResult({ ...correction, originalId: '10' }, document, 'correction', target))
  const reversal = reversalDocument(posted, '2026-10-24'); assert.equal(reversal.lines[0]!.side, 'credit'); assert.equal(reversal.lines[0]!.memo, null); assert.equal(reversal.explanation, null)
})
test('draft lists reject duplicates and excessive counts and posted links reject self/missing explanation', () => {
  assert.throws(() => parseDraftList([wire(), wire()])); assert.throws(() => parseDraftList(Array(5001).fill(wire())))
  const source = { ...(wire() as object), relationship: 'correction', original_entry_id: '8' }
  assert.throws(() => parseJournalEntry(source, false))
  assert.throws(() => parseJournalEntry({ ...source, original_entry_id: '9', explanation: ' ' }, false))
})

test('optional entry time normalizes native input and roundtrips strict wire seconds', () => {
  for (const [time, expected] of [[undefined, null], ['', null], ['00:00', '00:00:00'], ['23:59:59', '23:59:59']] as const) {
    const document = journalDocument({ ...input(), time }, true, active)
    assert.equal(document.entry_time, expected)
    const entry = parseJournalEntry({ ...(wire('1') as object), entry_time: expected }, true)
    assert.equal(entry.input.time, expected ?? '')
    assert.equal(entryDocument(entry).entry_time, expected)
  }
  assert.equal(entryDocument(parseJournalEntry(wire(), true)).entry_time, null)
  for (const time of ['24:00:00', '12:60:00', '12:00:60', '1:02:03', ' 12:00:00', '12:00:00Z', '12:00:00.1']) {
    assert.throws(() => journalDocument({ ...input(), time }, false, active), time)
    assert.throws(() => parseJournalEntry({ ...(wire() as object), entry_time: time }, true), time)
  }
  for (const time of ['', '12:30', 123, false]) assert.throws(() => parseJournalEntry({ ...(wire() as object), entry_time: time }, true))
})

test('save verification rejects missing or changed time and reversal time is independently selected', () => {
  const target = parseJournalEntry({ ...(wire('1') as object), entry_time: '09:10:11' }, true)
  const expected = entryDocument(target), saved = { ...target, revision: '2' }
  assert.doesNotThrow(() => verifyJournalResult(saved, expected, 'replace', target))
  for (const time of ['', undefined, '09:10:12']) assert.throws(() => verifyJournalResult({ ...saved, input: { ...saved.input, time } }, expected, 'replace', target))
  assert.equal(reversalDocument(target, '2026-10-24').entry_time, null)
  assert.equal(reversalDocument(target, '2026-10-24', '15:30').entry_time, '15:30:00')
  assert.equal(target.input.time, '09:10:11')
})
