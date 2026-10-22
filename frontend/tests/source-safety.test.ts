import assert from 'node:assert/strict'
import { readFileSync, readdirSync } from 'node:fs'
import test from 'node:test'

const sourceDirectory = new URL('../src/', import.meta.url)
const indexSource = readFileSync(new URL('../index.html', import.meta.url), 'utf8')
const mainSource = readFileSync(new URL('main.ts', sourceDirectory), 'utf8')
const frontendTypeScriptSource = readdirSync(sourceDirectory, { recursive: true })
  .filter((name) => name.endsWith('.ts'))
  .map((name) => readFileSync(new URL(name, sourceDirectory), 'utf8'))
  .join('\n')
const frontendMarkupSource = `${indexSource}\n${frontendTypeScriptSource}`

function sourceBetween(start: string, end: string): string {
  const startIndex = mainSource.indexOf(start)
  const endIndex = mainSource.indexOf(end, startIndex + start.length)
  assert.notEqual(startIndex, -1, `missing source marker: ${start}`)
  assert.notEqual(endIndex, -1, `missing source marker: ${end}`)
  return mainSource.slice(startIndex, endIndex)
}

function assertAppearsInOrder(source: string, markers: string[]): void {
  let previousIndex = -1
  for (const marker of markers) {
    const index = source.indexOf(marker, previousIndex + 1)
    assert.notEqual(index, -1, `missing source marker: ${marker}`)
    assert.ok(index > previousIndex, `source marker is out of order: ${marker}`)
    previousIndex = index
  }
}

const initialShell = sourceBetween('app.innerHTML = `', '`\n\nconst shell')
const runtimeRendering = mainSource.slice(
  mainSource.indexOf('`\n\nconst shell') + '`\n\nconst shell'.length,
)

test('main source keeps a single explicit read-only account-tree request contract', () => {
  const request = sourceBetween('const response = await fetch', 'if (!response.ok)')

  assert.equal(frontendTypeScriptSource.match(/\bfetch\s*\(/g)?.length, 1)
  assert.match(request, /fetch\(\s*['"]\/api\/accounts\/tree['"]/)
  assert.match(request, /method:\s*['"]GET['"]/)
  assert.match(request, /headers:\s*\{\s*Accept:\s*['"]application\/json['"]\s*\}/)
  assert.match(request, /credentials:\s*['"]same-origin['"]/)
  assert.match(request, /cache:\s*['"]no-store['"]/)
  assert.match(request, /signal:\s*controller\.signal/)
  assert.doesNotMatch(
    frontendTypeScriptSource,
    /\b(?:XMLHttpRequest|WebSocket|EventSource)\b|sendBeacon\s*\(/,
  )
})

test('account responses are checked and parsed before becoming renderable state', () => {
  const loader = sourceBetween('async function loadAccounts', "refreshButton.addEventListener")

  assertAppearsInOrder(loader, [
    'if (!response.ok)',
    "response.headers.get('content-type')",
    "includes('application/json')",
    'const responseBody: unknown = await response.json()',
    'const nextAccountTree = parseAccountTree(responseBody)',
    'accountTree = nextAccountTree',
    'flattenAccountTree(accountTree, collapsedIds)',
    'renderAccounts()',
  ])
  assert.doesNotMatch(loader, /\b(?:error|response)\.(?:message|statusText)\b/)
})

test('API account names are written as text rather than interpreted as HTML', () => {
  const accountNameRenderer = sourceBetween(
    'function createAccountName',
    'function createStateCell',
  )

  assert.match(accountNameRenderer, /span\.textContent = segment\.text/)
  assert.doesNotMatch(accountNameRenderer, /innerHTML|insertAdjacentHTML|outerHTML/)
})

test('all runtime table content uses DOM text nodes rather than HTML parsing sinks', () => {
  const rowRenderer = sourceBetween('function renderAccountRow', 'function renderAccounts')

  assert.doesNotMatch(initialShell, /\$\{/)
  assert.equal(frontendTypeScriptSource.match(/\.innerHTML\s*=/g)?.length, 1)
  assert.doesNotMatch(
    frontendTypeScriptSource,
    /\.outerHTML\s*=|insertAdjacentHTML\s*\(|document\.write\s*\(|createContextualFragment\s*\(|new\s+DOMParser\b/,
  )
  assert.doesNotMatch(
    runtimeRendering,
    /\.innerHTML\s*=|\.outerHTML\s*=|insertAdjacentHTML\s*\(|document\.write\s*\(|createContextualFragment\s*\(|new\s+DOMParser\b/,
  )
  assert.match(rowRenderer, /elementCell\.textContent = elementLabels\[row\.account\.element\]/)
  assert.match(rowRenderer, /kindCell\.textContent =/)
  assert.match(rowRenderer, /heading\.textContent = title/)
  assert.match(rowRenderer, /description\.textContent = detail/)
  assert.match(rowRenderer, /button\.textContent = action\.label/)
})

test('tree disclosure retains arrow-key handling and restores focus after rendering', () => {
  const rowRenderer = sourceBetween('function renderAccountRow', 'function renderMessageRow')
  const collapseHandler = sourceBetween('function setAccountCollapsed', 'function setLoadingState')

  assert.match(rowRenderer, /event\.key === 'ArrowLeft'/)
  assert.match(rowRenderer, /event\.key === 'ArrowRight'/)
  assert.equal(rowRenderer.match(/event\.preventDefault\(\)/g)?.length, 2)
  assert.match(rowRenderer, /aria-expanded/)
  assert.match(collapseHandler, /renderAccounts\(\)/)
  assert.match(collapseHandler, /\.focus\(\)/)
})

test('refresh, retry, and disclosure controls retain a logical keyboard focus target', () => {
  const focusController = sourceBetween('function readFocusTarget', 'function setLoadingState')

  assert.match(focusController, /activeElement === refreshButton/)
  assert.match(focusController, /activeElement\.matches\('\.inline-button'\)/)
  assert.match(focusController, /activeElement\.closest<HTMLButtonElement>\('\.disclosure-button'\)/)
  assert.match(focusController, /document\.visibilityState !== 'visible'/)
  assert.match(focusController, /compactLayout\.matches && !sidebar\.hidden/)
  assert.match(focusController, /refreshButton\.focus\(\)/)
  assert.match(focusController, /if \(requestSucceeded\) tableFrame\.focus\(\)/)
  assert.match(focusController, /querySelector<HTMLButtonElement>\('\.inline-button'\)/)
  assert.match(
    focusController,
    /button\[data-account-id="\$\{target\.accountId\}"\]/,
  )
  assert.match(initialShell, /id="accounts-table-frame"[\s\S]*?tabindex="0"/)
})

test('compact sidebar supports Escape dismissal and returns focus to its toggle', () => {
  const sidebarController = sourceBetween(
    'const supportsNativeInert',
    'function createAccountName',
  )

  assert.match(sidebarController, /event\.key === 'Escape'/)
  assert.match(sidebarController, /compactLayout\.matches && !sidebar\.hidden/)
  assertAppearsInOrder(sidebarController, [
    "sidebarScrim.addEventListener('click'",
    'setSidebarOpen(false)',
    'sidebarToggle.focus()',
  ])
  assert.match(
    sidebarController,
    /event\.key === 'Escape'[\s\S]*?setSidebarOpen\(false\)[\s\S]*?sidebarToggle\.focus\(\)/,
  )
  assert.match(sidebarController, /mainContent\.inert = (?:blocked|overlayOpen)/)
  assert.match(sidebarController, /mainContent\.setAttribute\('aria-hidden', 'true'\)/)
})

test('aborted or superseded loads cannot publish an error or clear latest-request busy state', () => {
  const loader = sourceBetween('async function loadAccounts', "refreshButton.addEventListener")
  const catchIndex = loader.indexOf('} catch (error) {')
  const errorStateIndex = loader.indexOf("statusLine.className = 'status-line is-error'", catchIndex)
  const staleRequestGuards =
    loader.match(
      /if \((?:controller\.signal\.aborted|!accountRequests\.isCurrent\(controller\))\) return/g,
    ) ?? []
  assert.notEqual(catchIndex, -1)
  assert.notEqual(errorStateIndex, -1)
  const preErrorCatch = loader.slice(catchIndex, errorStateIndex)

  assert.ok(staleRequestGuards.length >= 3)
  assert.match(
    loader,
    /const nextAccountTree = parseAccountTree\(responseBody\)[\s\S]*?if \(!accountRequests\.isCurrent\(controller\)\) return[\s\S]*?accountTree = nextAccountTree/,
  )
  assert.match(preErrorCatch, /if \(!accountRequests\.isCurrent\(controller\)\) return/)
  assert.match(preErrorCatch, /error\.name === 'AbortError'\) return/)
  assertAppearsInOrder(loader, [
    'setLoadingState(preserveContent)',
    'const response = await fetch',
    "error.name === 'AbortError') return",
    '} finally {',
    'if (accountRequests.finish(controller))',
    'refreshButton.disabled = false',
    "tableFrame.removeAttribute('aria-busy')",
  ])
})

test('page lifecycle wiring cancels hidden work and refreshes a restored BFCache page', () => {
  assert.match(
    mainSource,
    /window\.addEventListener\('pagehide', \(\) => accountRequests\.cancel\(\)\)/,
  )
  assert.match(
    mainSource,
    /window\.addEventListener\('pageshow', \(event\) => \{[\s\S]*?if \(event\.persisted\) void loadAccounts\(\)/,
  )
})

test('frontend source does not expose a mutation transport or unapproved editor surface', () => {
  assert.doesNotMatch(
    frontendTypeScriptSource,
    /method:\s*['"](?:POST|PUT|PATCH|DELETE)['"]|\bbody\s*:|new\s+FormData\b|sendBeacon\s*\(/,
  )
  assert.doesNotMatch(
    frontendMarkupSource,
    /<\/?form\b|<input\b|<textarea\b|<select\b|contenteditable|designMode|execCommand|journal-entry-editor/i,
  )
  assert.match(indexSource, /<title>kAccounting-Extra-01<\/title>/)
  assert.match(initialShell, /<h1>kAccounting-Extra-01<\/h1>/)
  assert.match(initialShell, /本機唯讀介面/)
  assert.match(initialShell, /僅讀取本機 API；尚未開放資料新增或修改。/)
  assert.match(initialShell, /餘額須由已過帳分錄產生；目前唯讀 API 未提供各科目餘額。/)
  assert.match(initialShell, /<button class="nav-item" type="button" disabled title="尚未開放">[\s\S]*?日記簿/)
  assert.match(
    initialShell,
    /<button class="primary-button" type="button" disabled title="新增功能尚未開放">新增科目<\/button>/,
  )
})
