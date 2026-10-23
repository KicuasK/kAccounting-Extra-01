import assert from 'node:assert/strict'
import test from 'node:test'
import { nativeDownloadUrl, requestJson } from '../src/session-client.ts'

test('large upload body stays the original File with pinned generation', async () => {
  const file = new File(['fixture'], 'receipt.txt')
  const original = globalThis.fetch
  globalThis.fetch = (async (_path, options) => {
    assert.equal(options?.body, file)
    assert.equal(new Headers(options?.headers).get('content-type'), 'application/octet-stream')
    assert.equal(new Headers(options?.headers).get('x-kaccounting-selection'), 'a'.repeat(32))
    return new Response('{}', { headers: { 'Content-Type': 'application/json' } })
  }) as typeof fetch
  try { await requestJson('/api/journal/entries/1/attachments', new AbortController().signal, 'POST', file, 'a'.repeat(32)) }
  finally { globalThis.fetch = original }
})
test('native downloads accept only exact binary routes and generation', () => {
  assert.equal(nativeDownloadUrl('/api/attachments/abcdefabcdefabcdefabcdefabcdefab/content', 'a'.repeat(32)), `/api/attachments/abcdefabcdefabcdefabcdefabcdefab/content?download_selection=${'a'.repeat(32)}`)
  assert.match(nativeDownloadUrl(`/api/storage/operations/${'b'.repeat(32)}/download`, 'a'.repeat(32)), /download_selection=/)
  for (const path of ['https://evil.test', '/api/accounts', '/api/attachments/abcdefabcdefabcdefabcdefabcdefab/content?other=1', '/api/attachments/abcdefabcdefabcdefabcdefabcdefab/content#x', '/api/attachments/../content']) assert.throws(() => nativeDownloadUrl(path, 'a'.repeat(32)))
  assert.throws(() => nativeDownloadUrl('/api/attachments/abcdefabcdefabcdefabcdefabcdefab/content', 'unselected'))
})
