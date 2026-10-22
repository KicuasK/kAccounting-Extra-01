import './style.css'
import {
  AccountTreeResponseError,
  flattenAccountTree,
  parseAccountTree,
  segmentAccountName,
  type AccountNode,
  type AccountingElement,
  type FlatAccountNode,
} from './account-tree'
import { LatestRequestGate } from './latest-request'

const app = document.querySelector<HTMLDivElement>('#app')
if (!app) throw new Error('找不到應用程式根節點')

app.innerHTML = `
  <div class="app-shell">
    <header class="toolbar">
      <button class="toolbar-button sidebar-toggle" id="sidebar-toggle" type="button"
        aria-controls="primary-sidebar" aria-expanded="true" aria-label="收合側邊欄">
        <span class="sidebar-icon" aria-hidden="true"><i></i><i></i><i></i></span>
      </button>
      <h1>kAccounting-Extra-01</h1>
      <span class="environment-status"><span aria-hidden="true"></span>本機唯讀介面</span>
    </header>

    <div class="workspace">
      <aside class="sidebar" id="primary-sidebar" aria-label="主要導覽">
        <p class="sidebar-heading">帳務</p>
        <nav>
          <button class="nav-item is-selected" type="button" aria-current="page">
            <span class="nav-symbol" aria-hidden="true">▤</span>會計科目
          </button>
          <button class="nav-item" type="button" disabled title="尚未開放">
            <span class="nav-symbol" aria-hidden="true">≡</span>日記簿
          </button>
          <button class="nav-item" type="button" disabled title="尚未開放">
            <span class="nav-symbol" aria-hidden="true">☷</span>總分類帳
          </button>
          <button class="nav-item" type="button" disabled title="尚未開放">
            <span class="nav-symbol" aria-hidden="true">⊞</span>試算表
          </button>
        </nav>
        <p class="sidebar-heading report-heading">報表</p>
        <nav>
          <button class="nav-item" type="button" disabled title="尚未開放">
            <span class="nav-symbol" aria-hidden="true">▧</span>資產負債表
          </button>
          <button class="nav-item" type="button" disabled title="尚未開放">
            <span class="nav-symbol" aria-hidden="true">▥</span>收支報表
          </button>
        </nav>
        <p class="sidebar-note">僅讀取本機 API；尚未開放資料新增或修改。</p>
      </aside>
      <button class="sidebar-scrim" id="sidebar-scrim" type="button" aria-label="關閉側邊欄" hidden></button>

      <main class="content" id="main-content">
        <section class="content-header" aria-labelledby="page-title">
          <div>
            <p class="eyebrow">科目管理</p>
            <h2 id="page-title">會計科目</h2>
          </div>
          <div class="header-actions">
            <button class="secondary-button" id="refresh-button" type="button">重新整理</button>
            <button class="primary-button" type="button" disabled title="新增功能尚未開放">新增科目</button>
          </div>
        </section>

        <div class="content-body">
          <div class="status-line" id="status-line" role="status" aria-live="polite">
            <span id="status-message">正在讀取科目…</span>
          </div>

          <section class="table-frame" id="accounts-table-frame" aria-labelledby="accounts-caption"
            tabindex="0">
            <table aria-describedby="balance-note">
              <caption id="accounts-caption">會計科目清單</caption>
              <colgroup>
                <col class="account-column" />
                <col class="element-column" />
                <col class="kind-column" />
                <col class="state-column" />
                <col class="balance-column" />
              </colgroup>
              <thead>
                <tr>
                  <th scope="col">科目名稱</th>
                  <th scope="col">會計要素</th>
                  <th scope="col">節點類型</th>
                  <th scope="col">狀態</th>
                  <th scope="col" class="amount-column">餘額（TWD）</th>
                </tr>
              </thead>
              <tbody id="accounts-body"></tbody>
            </table>
          </section>

          <footer class="table-footer">
            <span id="account-count" class="numeric">0 個科目</span>
            <span id="balance-note">餘額須由已過帳分錄產生；目前唯讀 API 未提供各科目餘額。</span>
          </footer>
        </div>
      </main>
    </div>
  </div>
`

const shell = document.querySelector<HTMLDivElement>('.app-shell')!
const sidebar = document.querySelector<HTMLElement>('#primary-sidebar')!
const sidebarToggle = document.querySelector<HTMLButtonElement>('#sidebar-toggle')!
const sidebarScrim = document.querySelector<HTMLButtonElement>('#sidebar-scrim')!
const mainContent = document.querySelector<HTMLElement>('#main-content')!
const refreshButton = document.querySelector<HTMLButtonElement>('#refresh-button')!
const tableFrame = document.querySelector<HTMLElement>('#accounts-table-frame')!
const accountsBody = document.querySelector<HTMLTableSectionElement>('#accounts-body')!
const statusLine = document.querySelector<HTMLDivElement>('#status-line')!
const statusMessage = document.querySelector<HTMLSpanElement>('#status-message')!
const accountCount = document.querySelector<HTMLSpanElement>('#account-count')!
const compactLayout = window.matchMedia('(max-width: 900px), (pointer: coarse)')
const supportsNativeInert = 'inert' in HTMLElement.prototype

const elementLabels: Record<AccountingElement, string> = {
  asset: '資產',
  liability: '負債',
  equity: '權益',
  income: '收入',
  expense: '費用',
}

let accountTree: AccountNode[] = []
let hasSuccessfulAccountResponse = false
const accountRequests = new LatestRequestGate()
const collapsedIds = new Set<string>()

type FocusTarget =
  | { kind: 'account'; accountId: string }
  | { kind: 'refresh' }
  | { kind: 'retry' }
  | null

function setMainContentBlocked(blocked: boolean): void {
  mainContent.inert = blocked
  if (
    !supportsNativeInert &&
    blocked &&
    mainContent.contains(document.activeElement)
  ) {
    // Safari 13 does not implement inert. The focusin guard below keeps focus
    // outside the hidden content; move an already-focused control immediately.
    sidebarToggle.focus()
  }
  if (blocked) mainContent.setAttribute('aria-hidden', 'true')
  else mainContent.removeAttribute('aria-hidden')
}

function setSidebarOpen(open: boolean): void {
  sidebar.hidden = !open
  sidebarToggle.setAttribute('aria-expanded', String(open))
  sidebarToggle.setAttribute('aria-label', open ? '收合側邊欄' : '展開側邊欄')
  shell.classList.toggle('sidebar-is-open', open)
  const overlayOpen = compactLayout.matches && open
  sidebarScrim.hidden = !overlayOpen
  setMainContentBlocked(overlayOpen)
}

function handleLayoutChange(): void {
  setSidebarOpen(!compactLayout.matches)
}

sidebarToggle.addEventListener('click', () => {
  setSidebarOpen(sidebar.hidden !== false)
})
sidebarScrim.addEventListener('click', () => {
  setSidebarOpen(false)
  sidebarToggle.focus()
})
mainContent.addEventListener('focusin', () => {
  if (compactLayout.matches && !sidebar.hidden) sidebarToggle.focus()
})
document.addEventListener('keydown', (event) => {
  if (event.key === 'Escape' && compactLayout.matches && !sidebar.hidden) {
    setSidebarOpen(false)
    sidebarToggle.focus()
  }
})
if (typeof compactLayout.addEventListener === 'function') {
  compactLayout.addEventListener('change', handleLayoutChange)
} else {
  compactLayout.addListener(handleLayoutChange)
}
handleLayoutChange()

function createAccountName(name: string): HTMLSpanElement {
  const container = document.createElement('span')
  container.className = 'account-name'
  container.dir = 'auto'

  segmentAccountName(name).forEach((segment) => {
    const span = document.createElement('span')
    if (segment.script === 'han') span.className = 'han'
    span.textContent = segment.text
    container.append(span)
  })

  return container
}

function createStateCell(isActive: boolean): HTMLTableCellElement {
  const cell = document.createElement('td')
  const state = document.createElement('span')
  state.className = `account-state ${isActive ? 'is-active' : 'is-inactive'}`
  const dot = document.createElement('span')
  dot.className = 'state-dot'
  dot.setAttribute('aria-hidden', 'true')
  state.append(dot, isActive ? '啟用' : '停用')
  cell.append(state)
  return cell
}

function renderAccountRow(row: FlatAccountNode): HTMLTableRowElement {
  const tableRow = document.createElement('tr')
  tableRow.dataset.accountId = row.account.id
  if (!row.account.isActive) tableRow.classList.add('is-inactive')

  const nameCell = document.createElement('th')
  nameCell.scope = 'row'
  const nameLayout = document.createElement('div')
  nameLayout.className = 'account-cell'
  nameLayout.style.setProperty('--tree-depth', String(row.depth))

  if (row.hasChildren) {
    const disclosure = document.createElement('button')
    const isCollapsed = collapsedIds.has(row.account.id)
    disclosure.type = 'button'
    disclosure.className = 'disclosure-button'
    disclosure.dataset.accountId = row.account.id
    disclosure.setAttribute('aria-expanded', String(!isCollapsed))
    disclosure.setAttribute(
      'aria-label',
      `${isCollapsed ? '展開' : '收合'}「${row.account.name}」`,
    )
    disclosure.textContent = isCollapsed ? '▸' : '▾'
    disclosure.addEventListener('click', () => setAccountCollapsed(row.account.id, !isCollapsed))
    disclosure.addEventListener('keydown', (event) => {
      if (event.key === 'ArrowLeft' && !isCollapsed) {
        event.preventDefault()
        setAccountCollapsed(row.account.id, true)
      } else if (event.key === 'ArrowRight' && isCollapsed) {
        event.preventDefault()
        setAccountCollapsed(row.account.id, false)
      }
    })
    nameLayout.append(disclosure)
  } else {
    const spacer = document.createElement('span')
    spacer.className = 'disclosure-spacer'
    spacer.setAttribute('aria-hidden', 'true')
    nameLayout.append(spacer)
  }

  nameLayout.append(createAccountName(row.account.name))
  nameCell.append(nameLayout)

  const elementCell = document.createElement('td')
  elementCell.textContent = elementLabels[row.account.element]

  const kindCell = document.createElement('td')
  kindCell.textContent = row.account.kind === 'category' ? '分類' : '過帳科目'

  const balanceCell = document.createElement('td')
  balanceCell.className = 'amount-column amount-unavailable'
  balanceCell.textContent = '—'
  balanceCell.setAttribute('aria-label', '尚未提供餘額')

  tableRow.append(
    nameCell,
    elementCell,
    kindCell,
    createStateCell(row.account.isActive),
    balanceCell,
  )
  return tableRow
}

function renderMessageRow(
  title: string,
  detail: string,
  className: string,
  action?: { label: string; run: () => void },
): void {
  const row = document.createElement('tr')
  row.className = className
  const cell = document.createElement('td')
  cell.colSpan = 5
  const heading = document.createElement('strong')
  heading.textContent = title
  const description = document.createElement('span')
  description.textContent = detail
  cell.append(heading, description)
  if (action) {
    const button = document.createElement('button')
    button.type = 'button'
    button.className = 'secondary-button inline-button'
    button.textContent = action.label
    button.addEventListener('click', action.run)
    cell.append(button)
  }
  row.append(cell)
  accountsBody.textContent = ''
  accountsBody.append(row)
}

function renderAccounts(): void {
  const rows = flattenAccountTree(accountTree, collapsedIds)
  const fragment = document.createDocumentFragment()
  rows.forEach((row) => fragment.append(renderAccountRow(row)))
  accountsBody.textContent = ''
  accountsBody.append(fragment)
}

function pruneCollapsedAccounts(nodes: AccountNode[]): number {
  const expandableIds = new Set<string>()
  const pending = [...nodes]
  let totalAccountCount = 0

  while (pending.length > 0) {
    const node = pending.pop()!
    totalAccountCount += 1
    if (node.children.length > 0) expandableIds.add(node.account.id)
    node.children.forEach((child) => pending.push(child))
  }

  collapsedIds.forEach((accountId) => {
    if (!expandableIds.has(accountId)) collapsedIds.delete(accountId)
  })

  return totalAccountCount
}

function setAccountCollapsed(accountId: string, collapsed: boolean): void {
  if (collapsed) collapsedIds.add(accountId)
  else collapsedIds.delete(accountId)
  renderAccounts()
  accountsBody
    .querySelector<HTMLButtonElement>(`button[data-account-id="${accountId}"]`)
    ?.focus()
}

function readFocusTarget(): FocusTarget {
  const activeElement = document.activeElement
  if (!(activeElement instanceof HTMLElement)) return null
  if (activeElement === refreshButton) return { kind: 'refresh' }
  if (activeElement.matches('.inline-button')) return { kind: 'retry' }

  const disclosure = activeElement.closest<HTMLButtonElement>('.disclosure-button')
  const accountId = disclosure?.dataset.accountId
  return accountId ? { kind: 'account', accountId } : null
}

function reconcileFocusTarget(previousTarget: FocusTarget): FocusTarget {
  const currentTarget = readFocusTarget()
  if (currentTarget) return currentTarget

  const activeElement = document.activeElement
  const focusFellBackToDocument =
    activeElement === null ||
    activeElement === document.body ||
    activeElement === document.documentElement
  return focusFellBackToDocument ? previousTarget : null
}

function restoreFocus(target: FocusTarget, requestSucceeded: boolean): void {
  if (!target || document.visibilityState !== 'visible') return
  if (compactLayout.matches && !sidebar.hidden) return

  if (target.kind === 'refresh') {
    refreshButton.focus()
  } else if (target.kind === 'retry') {
    if (requestSucceeded) tableFrame.focus()
    else accountsBody.querySelector<HTMLButtonElement>('.inline-button')?.focus()
  } else {
    accountsBody
      .querySelector<HTMLButtonElement>(`button[data-account-id="${target.accountId}"]`)
      ?.focus()
  }
}

function setLoadingState(preserveContent: boolean): void {
  refreshButton.disabled = true
  tableFrame.setAttribute('aria-busy', 'true')
  statusLine.className = 'status-line is-loading'
  statusMessage.textContent = preserveContent ? '正在更新科目…' : '正在讀取科目…'
  if (!preserveContent) {
    accountCount.textContent = '0 個科目'
    renderMessageRow('正在讀取科目', '請稍候。', 'message-row loading-row')
  }
}

async function loadAccounts(): Promise<void> {
  let focusTarget = readFocusTarget()
  const preserveContent = hasSuccessfulAccountResponse
  const controller = accountRequests.begin()
  let requestSucceeded = false
  setLoadingState(preserveContent)

  try {
    const response = await fetch('/api/accounts/tree', {
      method: 'GET',
      headers: { Accept: 'application/json' },
      credentials: 'same-origin',
      cache: 'no-store',
      signal: controller.signal,
    })
    if (controller.signal.aborted) return
    if (!accountRequests.isCurrent(controller)) return
    if (!response.ok) throw new Error(`HTTP ${response.status}`)
    const contentType = response.headers.get('content-type') ?? ''
    if (!contentType.toLowerCase().includes('application/json')) {
      throw new Error('回應不是 JSON')
    }

    const responseBody: unknown = await response.json()
    if (controller.signal.aborted) return
    if (!accountRequests.isCurrent(controller)) return
    const nextAccountTree = parseAccountTree(responseBody)
    if (controller.signal.aborted) return
    if (!accountRequests.isCurrent(controller)) return

    focusTarget = reconcileFocusTarget(focusTarget)
    // Keep parsing local until the latest-request check above. This is the safe
    // equivalent of `accountTree = parseAccountTree(await response.json())`.
    accountTree = nextAccountTree
    hasSuccessfulAccountResponse = true
    const totalAccountCount = pruneCollapsedAccounts(accountTree)
    const rows = flattenAccountTree(accountTree, collapsedIds)
    accountCount.textContent = `${totalAccountCount} 個科目`
    statusLine.className = 'status-line is-ready'
    statusMessage.textContent = accountTree.length === 0 ? '尚未建立科目' : '科目資料已更新'

    if (rows.length === 0) {
      renderMessageRow(
        '尚未建立科目資料',
        '新增功能會在互動設計與寫入邊界核准後開放。',
        'message-row empty-row',
      )
    } else {
      renderAccounts()
    }
    requestSucceeded = true
  } catch (error) {
    if (error instanceof Error && error.name === 'AbortError') return
    if (!accountRequests.isCurrent(controller)) return
    focusTarget = reconcileFocusTarget(focusTarget)
    statusLine.className = 'status-line is-error'

    if (preserveContent) {
      statusMessage.textContent =
        error instanceof AccountTreeResponseError
          ? '更新回應未通過安全檢查；仍顯示上次資料'
          : '無法更新科目；仍顯示上次資料'
    } else {
      accountTree = []
      accountCount.textContent = '0 個科目'
      if (error instanceof AccountTreeResponseError) {
        statusMessage.textContent = '科目回應未通過安全檢查'
        renderMessageRow(
          '無法安全顯示科目資料',
          '本機服務傳回的科目樹格式不一致；介面已停止顯示且不會修改資料。',
          'message-row error-row',
          { label: '重新嘗試', run: () => void loadAccounts() },
        )
      } else {
        statusMessage.textContent = '無法讀取本機科目資料'
        renderMessageRow(
          '本機科目資料目前無法使用',
          '請確認本機服務可用後再重新整理；介面不會建立或修改任何資料。',
          'message-row error-row',
          { label: '重新嘗試', run: () => void loadAccounts() },
        )
      }
    }
  } finally {
    if (accountRequests.finish(controller)) {
      refreshButton.disabled = false
      tableFrame.removeAttribute('aria-busy')
      restoreFocus(focusTarget, requestSucceeded)
    }
  }
}

refreshButton.addEventListener('click', () => void loadAccounts())
window.addEventListener('pagehide', () => accountRequests.cancel())
window.addEventListener('pageshow', (event) => {
  if (event.persisted) void loadAccounts()
})
void loadAccounts()
