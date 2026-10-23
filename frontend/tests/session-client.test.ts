import assert from 'node:assert/strict'
import test from 'node:test'
import { ApiError, apiGuidance, parseBooks, parseSession, requestJson, validBookId } from '../src/session-client.ts'

test('session parsing requires explicit consistent selection state', () => {
  assert.deepEqual(parseSession({ book_id: null, selection_required: true, selection_id: null }), { bookId: null, selectionRequired: true, selectionId: null })
  assert.deepEqual(parseSession({ book_id: '書簿 A', selection_required: false, selection_id: 'a'.repeat(32) }), { bookId: '書簿 A', selectionRequired: false, selectionId: 'a'.repeat(32) })
  for (const value of [null, {}, { book_id: null, selection_required: false }, { book_id: 'book', selection_required: true }, { book_id: 1, selection_required: false }]) assert.throws(() => parseSession(value))
})
test('book discovery rejects path syntax, controls, duplicates and untrusted status', () => {
  for (const value of ['', '.hidden', '../book', '/book', 'a/b', 'a\\b', 'a\n', 1]) assert.equal(validBookId(value), false)
  assert.deepEqual(parseBooks({ books: [{ id: 'book', status: 'selection_required' }] }), ['book'])
  for (const books of [[{ id: 'book', status: 'ready' }], [{ id: 'book', status: 'selection_required' }, { id: 'book', status: 'selection_required' }]]) assert.throws(() => parseBooks({ books }))
})
test('transport uses exact same-origin no-store JSON and abort signal; transitions are explicit', async () => {
  const original = globalThis.fetch
  const calls: { path: unknown; init: RequestInit | undefined }[] = []
  globalThis.fetch = (async (path, init) => { calls.push({ path, init }); return Response.json({ book_id: 'book', selection_required: false }) }) as typeof fetch
  try {
    const controller = new AbortController()
    await requestJson('/api/session', controller.signal)
    await requestJson('/api/session/selection', controller.signal, 'POST', { book_id: 'book' })
    await requestJson('/api/session/selection', controller.signal, 'DELETE')
    await requestJson('/api/accounts/tree', controller.signal, 'GET', undefined, 'a'.repeat(32))
    assert.deepEqual(calls.map(call => call.init!.method), ['GET', 'POST', 'DELETE', 'GET'])
    for (const call of calls) {
      assert.equal(call.init!.credentials, 'same-origin'); assert.equal(call.init!.cache, 'no-store')
      assert.equal(call.init!.redirect, 'error'); assert.equal(call.init!.signal, controller.signal)
    }
    assert.equal(calls[0]!.init!.body, undefined)
    assert.equal(calls[1]!.init!.body, '{"book_id":"book"}')
    assert.equal((calls[3]!.init!.headers as Record<string, string>)['X-kAccounting-Selection'], 'a'.repeat(32))
  } finally { globalThis.fetch = original }
})
test('missing, malformed and mismatched selection generations are refused before data loads', () => {
  for (const selection_id of [undefined, null, '', 1, 'A'.repeat(32), 'a'.repeat(31), 'a'.repeat(33)]) {
    assert.throws(() => parseSession({ book_id: 'book', selection_required: false, selection_id }))
  }
  assert.throws(() => parseSession({ book_id: null, selection_required: true, selection_id: 'a'.repeat(32) }))
})
test('HTTP errors accept both envelopes but show static local guidance rather than server text', async () => {
  const original = globalThis.fetch
  try {
    for (const body of [{ code: 'book_busy', message: '<script>secret path</script>' }, { error: { code: 'book_busy' } }]) {
      globalThis.fetch = (async () => Response.json(body, { status: 409 })) as typeof fetch
      await assert.rejects(requestJson('/api/session', new AbortController().signal), error => {
        assert.ok(error instanceof ApiError)
        assert.match(apiGuidance(error), /處理中的作業/)
        assert.doesNotMatch(apiGuidance(error), /script|secret/)
        return true
      })
    }
    globalThis.fetch = (async () => new Response('<html>login</html>', { headers: { 'Content-Type': 'text/html' } })) as typeof fetch
    await assert.rejects(requestJson('/api/session', new AbortController().signal))
    assert.doesNotMatch(apiGuidance(new ApiError('unknown /private/secret')), /private|secret/)
  } finally { globalThis.fetch = original }
})

test('backup identity guidance separates explicit initialization, existing identity and corruption', () => {
  assert.match(apiGuidance(new ApiError('backup_identity_missing', 409)), /明確完成備份身分設定/)
  assert.match(apiGuidance(new ApiError('backup_identity_exists', 409)), /未被替換/)
  assert.match(apiGuidance(new ApiError('backup_identity_invalid', 409)), /不會覆寫/)
  assert.match(apiGuidance(new ApiError('backup_identity_initialization_failed', 500)), /不要重複提交/)
})
