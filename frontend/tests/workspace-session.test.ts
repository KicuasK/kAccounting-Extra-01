import assert from 'node:assert/strict'
import test from 'node:test'
import { ApiError, requestJson } from '../src/session-client.ts'
import { pinnedWorkspaceRequest } from '../src/workspace-session.ts'

const original = { bookId: 'book', selectionId: 'a'.repeat(32) }
test('mounted requests pin generation and refuse stale/ABA target changes before transport', async () => {
  let current = original
  const calls: unknown[][] = []
  const request = pinnedWorkspaceRequest(original, () => current, async (...args) => { calls.push(args); return {} }, () => {})
  const signal = new AbortController().signal
  await request('/api/accounts', signal)
  assert.equal(calls[0]![4], original.selectionId)
  current = { ...original, selectionId: 'b'.repeat(32) }
  await assert.rejects(request('/api/accounts', signal, 'POST', {}), error => error instanceof ApiError && error.code === 'selection_changed')
  assert.equal(calls.length, 1)
})
test('write admission spans response decoding, prevents duplicates and always releases on failure', async () => {
  let finish: ((value: unknown) => void) | undefined
  const changes: number[] = []
  const request = pinnedWorkspaceRequest(original, () => original, () => new Promise(resolve => { finish = resolve }), delta => changes.push(delta))
  const signal = new AbortController().signal
  const pending = request('/api/journal/drafts', signal, 'POST', {})
  await assert.rejects(request('/api/journal/drafts', signal, 'POST', {}), /無法完成/)
  assert.deepEqual(changes, [1])
  finish!(null); await pending
  assert.deepEqual(changes, [1, -1])
  const failing = pinnedWorkspaceRequest(original, () => original, async () => { throw new Error('fixture') }, delta => changes.push(delta))
  await assert.rejects(failing('/api/accounts/1', signal, 'DELETE', {}))
  assert.deepEqual(changes, [1, -1, 1, -1])
})
test('aborted and external/session-control paths never dispatch or mark a write pending', async () => {
  let calls = 0
  const request = pinnedWorkspaceRequest(original, () => original, async () => { calls++; return {} }, () => { calls++ })
  const controller = new AbortController(); controller.abort()
  await assert.rejects(request('/api/accounts', controller.signal, 'POST', {}), { name: 'AbortError' })
  for (const path of ['https://example.com', '//example.com', '/api/session', '/api/session/selection', '/api/accounts\\evil', '/api/accounts#fragment']) {
    await assert.rejects(request(path, new AbortController().signal))
  }
  assert.equal(calls, 0)
})
test('a decoded response cannot republish after local session invalidation', async () => {
  let current: typeof original | null = original
  const request = pinnedWorkspaceRequest(original, () => current, async () => { current = null; return { stale: true } }, () => {})
  await assert.rejects(request('/api/accounts', new AbortController().signal), error => error instanceof ApiError && error.code === 'selection_changed')
})
test('successful empty DELETE responses do not require JSON content type or body', async () => {
  const originalFetch = globalThis.fetch
  globalThis.fetch = (async () => new Response(null, { status: 204 })) as typeof fetch
  try { assert.equal(await requestJson('/api/accounts/1', new AbortController().signal, 'DELETE', { expected_revision: '1' }, original.selectionId), null) }
  finally { globalThis.fetch = originalFetch }
})
