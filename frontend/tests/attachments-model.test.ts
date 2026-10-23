import { test } from 'node:test'
import assert from 'node:assert/strict'
import { detachCommand, parseAttachment, parseAttachments, uploadPath, verifyAttachmentResult } from '../src/attachments-model.ts'
const metadata = { id: 'a'.repeat(32), entry_id: '9007199254740993', display_name: '收據.pdf', byte_length: '262144000', sha256: 'b'.repeat(64), created_at: 'now' }
test('attachment metadata keeps exact strings and refuses foreign ownership duplicate IDs or malformed integrity', () => {
  assert.equal(parseAttachment(metadata, metadata.entry_id).entryId, '9007199254740993')
  assert.throws(() => parseAttachment(metadata, '1')); assert.throws(() => parseAttachments([metadata, metadata], metadata.entry_id))
  for (const patch of [{ id: '1' }, { byte_length: 1 }, { byte_length: '262144001' }, { byte_length: '01' }, { sha256: 'x'.repeat(64) }, { display_name: '../receipt' }]) assert.throws(() => parseAttachment({ ...metadata, ...patch }, metadata.entry_id))
  assert.equal(parseAttachment({ ...metadata, byte_length: '0' }, metadata.entry_id).byteLength, '0')
})
test('upload uses UTF8 query regular-file metadata and exact draft revision; posted append omits revision', () => {
  const file = { name: '收據 #1.pdf', size: 262144000 }; const draft = uploadPath({ id: metadata.entry_id, revision: '9007199254740994' }, file)
  assert.match(draft, /display_name=%E6/); assert.match(draft, /source_kind=regular_file&expected_revision=9007199254740994$/)
  assert.ok(!uploadPath({ id: '1', revision: null }, file).includes('expected_revision'))
})
test('upload refuses oversize directory package executable and path filename before transport', () => {
  for (const file of [{ name: 'x', size: 262144001 }, { name: 'a.app', size: 1 }, { name: 'x.exe', size: 1 }, { name: 'script.sh', size: 1 }, { name: 'a.pdf', size: 1, webkitRelativePath: 'folder/a.pdf' }, { name: '../x', size: 1 }, { name: 'bad\u0000', size: 1 }, { name: 'a', size: 1.5 }]) assert.throws(() => uploadPath({ id: '1', revision: '1' }, file))
})
test('detach requires draft ownership and carries exact revision; verified response must identify attachment', () => {
  const item = parseAttachment(metadata, metadata.entry_id)
  assert.deepEqual(detachCommand({ id: item.entryId, revision: '9007199254740994' }, item).payload, { expected_revision: '9007199254740994' })
  assert.throws(() => detachCommand({ id: item.entryId, revision: null }, item)); assert.throws(() => detachCommand({ id: '1', revision: '1' }, item))
  assert.doesNotThrow(() => verifyAttachmentResult({ id: item.id, verified: true }, item.id)); assert.throws(() => verifyAttachmentResult({ id: 'c'.repeat(32), verified: true }, item.id)); assert.throws(() => verifyAttachmentResult({ id: item.id, verified: false }, item.id))
})
