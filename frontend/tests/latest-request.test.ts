import assert from 'node:assert/strict'
import test from 'node:test'

import { LatestRequestGate } from '../src/latest-request.ts'

test('starting a request aborts the previous request', () => {
  const gate = new LatestRequestGate()
  const previous = gate.begin()
  const latest = gate.begin()

  assert.equal(previous.signal.aborted, true)
  assert.equal(latest.signal.aborted, false)
  assert.equal(gate.isCurrent(previous), false)
  assert.equal(gate.isCurrent(latest), true)
})

test('an abort listener retry becomes current without leaving another request active', () => {
  const gate = new LatestRequestGate()
  const previous = gate.begin()
  let retry: AbortController | undefined
  previous.signal.addEventListener('abort', () => {
    retry = gate.begin()
  })

  const interrupted = gate.begin()

  assert.ok(retry)
  assert.equal(interrupted.signal.aborted, true)
  assert.equal(retry.signal.aborted, false)
  assert.equal(gate.isCurrent(interrupted), false)
  assert.equal(gate.isCurrent(retry), true)
})

test('multiple abort listener retries leave only the last retry active', () => {
  const gate = new LatestRequestGate()
  const previous = gate.begin()
  const retries: AbortController[] = []
  previous.signal.addEventListener('abort', () => retries.push(gate.begin()))
  previous.signal.addEventListener('abort', () => retries.push(gate.begin()))

  const interrupted = gate.begin()

  assert.equal(retries.length, 2)
  assert.equal(interrupted.signal.aborted, true)
  assert.equal(retries[0]!.signal.aborted, true)
  assert.equal(gate.isCurrent(retries[0]!), false)
  assert.equal(retries[1]!.signal.aborted, false)
  assert.equal(gate.isCurrent(retries[1]!), true)
})

test('a finishing abort listener cannot clear replacement ownership', () => {
  const gate = new LatestRequestGate()
  const previous = gate.begin()
  let staleFinishResult: boolean | undefined
  previous.signal.addEventListener('abort', () => {
    staleFinishResult = gate.finish(previous)
  })

  const latest = gate.begin()

  assert.equal(staleFinishResult, false)
  assert.equal(gate.isCurrent(latest), true)
})

test('a stale request cannot finish the latest request', () => {
  const gate = new LatestRequestGate()
  const stale = gate.begin()
  const latest = gate.begin()

  assert.equal(gate.finish(stale), false)
  assert.equal(gate.finish(latest), true)
  assert.equal(gate.isCurrent(latest), false)
  assert.equal(latest.signal.aborted, false)
  gate.cancel()
  assert.equal(latest.signal.aborted, false)
  assert.equal(gate.finish(latest), false)
})

test('cancelling aborts and clears the current request', () => {
  const gate = new LatestRequestGate()
  const current = gate.begin()

  gate.cancel()

  assert.equal(current.signal.aborted, true)
  assert.equal(gate.isCurrent(current), false)
  assert.equal(gate.finish(current), false)
})

test('cancelling repeatedly is idempotent and a later request can start', () => {
  const gate = new LatestRequestGate()
  const current = gate.begin()
  let abortCount = 0
  current.signal.addEventListener('abort', () => {
    abortCount += 1
  })

  gate.cancel()
  gate.cancel()
  const later = gate.begin()

  assert.equal(abortCount, 1)
  assert.equal(current.signal.aborted, true)
  assert.equal(gate.isCurrent(later), true)
})

test('a request started synchronously by a cancellation listener is also cancelled', () => {
  const gate = new LatestRequestGate()
  const cancelled = gate.begin()
  let retry: AbortController | undefined
  let cancelledFinishResult: boolean | undefined
  cancelled.signal.addEventListener('abort', () => {
    cancelledFinishResult = gate.finish(cancelled)
    gate.cancel()
    retry = gate.begin()
  })

  gate.cancel()

  assert.ok(retry)
  assert.equal(cancelledFinishResult, false)
  assert.equal(cancelled.signal.aborted, true)
  assert.equal(retry.signal.aborted, true)
  assert.equal(gate.isCurrent(retry), false)
  assert.equal(gate.finish(retry), false)
})

test('an externally aborted current request cannot apply a completion', () => {
  const gate = new LatestRequestGate()
  const current = gate.begin()

  current.abort()

  assert.equal(gate.isCurrent(current), false)
  assert.equal(gate.finish(current), true)
  assert.equal(gate.finish(current), false)
})

test('a stale completion is rejected even when abort did not stop its promise', async () => {
  const gate = new LatestRequestGate()
  let resolveStale!: () => void
  const staleCompletion = new Promise<void>((resolve) => {
    resolveStale = resolve
  })
  const applied: string[] = []
  const stale = gate.begin()
  const staleWorker = staleCompletion.then(() => {
    if (gate.isCurrent(stale)) applied.push('stale')
  })

  const latest = gate.begin()
  resolveStale()
  await staleWorker

  assert.deepEqual(applied, [])
  assert.equal(gate.isCurrent(latest), true)
})

test('a stale non-abort failure cannot publish an error', async () => {
  const gate = new LatestRequestGate()
  let rejectStale!: (error: Error) => void
  const staleFailure = new Promise<void>((_resolve, reject) => {
    rejectStale = reject
  })
  const publishedErrors: Error[] = []
  const stale = gate.begin()
  const staleWorker = staleFailure.catch((error: Error) => {
    if (gate.isCurrent(stale)) publishedErrors.push(error)
  })

  const latest = gate.begin()
  rejectStale(new Error('late network failure'))
  await staleWorker

  assert.deepEqual(publishedErrors, [])
  assert.equal(gate.isCurrent(latest), true)
})

test('pagehide cancellation invalidates completions before a BFCache restore retry', async () => {
  const gate = new LatestRequestGate()
  let resolveBeforePagehide!: () => void
  const lateCompletion = new Promise<void>((resolve) => {
    resolveBeforePagehide = resolve
  })
  const published: string[] = []
  const beforePagehide = gate.begin()
  const oldWorker = lateCompletion.then(() => {
    if (gate.isCurrent(beforePagehide)) published.push('before-pagehide')
  })

  gate.cancel()
  const afterPageshow = gate.begin()
  resolveBeforePagehide()
  await oldWorker
  if (gate.isCurrent(afterPageshow)) published.push('after-pageshow')

  assert.equal(beforePagehide.signal.aborted, true)
  assert.equal(gate.isCurrent(beforePagehide), false)
  assert.equal(gate.isCurrent(afterPageshow), true)
  assert.deepEqual(published, ['after-pageshow'])
  assert.equal(gate.finish(beforePagehide), false)
  assert.equal(gate.finish(afterPageshow), true)
})

test('rapid request replacement never lets an older request finish', () => {
  const gate = new LatestRequestGate()
  const staleRequests: AbortController[] = []

  for (let index = 0; index < 1_000; index += 1) {
    staleRequests.push(gate.begin())
  }
  const latest = gate.begin()

  staleRequests.forEach((request) => {
    assert.equal(request.signal.aborted, true)
    assert.equal(gate.isCurrent(request), false)
    assert.equal(gate.finish(request), false)
  })
  assert.equal(latest.signal.aborted, false)
  assert.equal(gate.isCurrent(latest), true)
  assert.equal(gate.finish(latest), true)
})

test('rapid retries publish only the latest result regardless of completion order', async () => {
  const gate = new LatestRequestGate()
  const published: number[] = []
  const pending = Array.from({ length: 100 }, (_, index) => {
    let resolve!: () => void
    const completion = new Promise<void>((done) => {
      resolve = done
    })
    const controller = gate.begin()
    const worker = completion
      .then(() => {
        if (gate.isCurrent(controller)) published.push(index)
      })
      .finally(() => gate.finish(controller))
    return { controller, resolve, worker }
  })

  for (let index = pending.length - 1; index >= 0; index -= 1) pending[index]!.resolve()
  await Promise.all(pending.map(({ worker }) => worker))

  assert.deepEqual(published, [pending.length - 1])
  assert.equal(gate.isCurrent(pending[pending.length - 1]!.controller), false)
})
