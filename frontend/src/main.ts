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
import { accountView } from './account-view'
import { accountBalances, formatBalance } from './account-balances'
import { ApiError, apiGuidance, parseBooks, parseSession, requestJson, startNativeDownload, validBookId } from './session-client'
import { mountAccountMaintenance } from './account-maintenance'
import { mountJournalUI } from './journal-ui'
import { mountReportsUI, type ReportView } from './reports-ui'
import { pinnedWorkspaceRequest, sameSnapshot, type SelectionSnapshot } from './workspace-session'
import { mountStorageUI } from './storage-ui'
import { mountPaymentMemosUI } from './payment-memos-ui'
import { mountOpenItemsUI } from './open-items-ui'

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
      <span class="environment-status" id="current-book">尚未選簿</span>
    </header>

    <div class="workspace">
      <aside class="sidebar" id="primary-sidebar" aria-label="主要導覽">
        <p class="sidebar-heading">帳務</p>
        <nav>
          <button class="nav-item is-selected" data-view="accounts" type="button" aria-current="page">
            <span class="nav-symbol" aria-hidden="true">▤</span>會計科目
          </button>
          <button class="nav-item" data-view="journal" type="button" disabled>
            <span class="nav-symbol" aria-hidden="true">＋</span>分錄編輯
          </button>
          <button class="nav-item" data-view="general-journal" type="button" disabled>
            <span class="nav-symbol" aria-hidden="true">≡</span>日記簿
          </button>
          <button class="nav-item" data-view="ledger" type="button" disabled>
            <span class="nav-symbol" aria-hidden="true">☷</span>總分類帳
          </button>
          <button class="nav-item" data-view="trial-balance" type="button" disabled>
            <span class="nav-symbol" aria-hidden="true">⊞</span>試算表
          </button>
        </nav>
        <p class="sidebar-heading report-heading">報表</p>
        <nav>
          <button class="nav-item" data-view="balance-sheet" type="button" disabled>
            <span class="nav-symbol" aria-hidden="true">▧</span>資產負債表
          </button>
          <button class="nav-item" data-view="income-expense" type="button" disabled>
            <span class="nav-symbol" aria-hidden="true">▥</span>收支報表
          </button>
        </nav>
        <nav><button class="nav-item" data-view="payment-memos" type="button" disabled><span class="nav-symbol" aria-hidden="true">◷</span>未來付款備忘錄</button></nav>
        <nav><button class="nav-item" data-view="open-items" type="button" disabled>應收／應付款</button></nav>
        <p class="sidebar-heading">資料管理</p>
        <nav><button class="nav-item" data-view="storage" type="button" disabled><span class="nav-symbol" aria-hidden="true">▣</span>備份與移轉</button></nav>
        <p class="sidebar-note">每次啟動先選擇帳簿。科目資料與餘額依目前帳簿讀取。</p>
      </aside>
      <button class="sidebar-scrim" id="sidebar-scrim" type="button" aria-label="關閉側邊欄" hidden></button>

      <main class="content" id="main-content">
        <section class="book-bar" aria-label="帳簿選擇">
          <label for="book-select">帳簿</label>
          <select id="book-select" disabled><option value="">請選擇帳簿</option></select>
          <button class="primary-button" id="open-book" type="button" disabled>開啟帳簿</button>
          <button class="secondary-button" id="close-book" type="button" disabled>關閉帳簿</button>
          <button class="secondary-button" id="create-book" type="button" disabled>建立帳簿…</button>
          <button class="secondary-button" id="refresh-books" type="button">更新清單</button>
          <span id="session-message" role="status" aria-live="polite">正在讀取帳簿清單…</span>
        </section>
        <section id="account-page" aria-label="科目瀏覽與維護">
        <section class="content-header" aria-labelledby="page-title">
          <div>
            <p class="eyebrow">科目管理</p>
            <h2 id="page-title">會計科目</h2>
          </div>
          <div class="header-actions">
            <button class="secondary-button" id="inactive-button" type="button" aria-pressed="false">停用科目</button>
            <button class="secondary-button" id="refresh-button" type="button">重新整理</button>
            <button class="primary-button" id="add-account" type="button" disabled>新增科目</button>
          </div>
        </section>

        <div class="content-body">
          <div class="filter-bar">
            <label class="search-label">搜尋科目<input id="account-search" type="search" placeholder="科目名稱或完整路徑" /></label>
            <label>科目性質<select id="element-filter"><option value="">全部</option><option value="asset">資產</option><option value="liability">負債</option><option value="equity">權益</option><option value="income">收益</option><option value="expense">費損</option></select></label>
            <label>餘額日期<input id="balance-date" type="date" /></label>
            <button class="secondary-button" id="reset-filter" type="button">重設篩選</button>
          </div>
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
                  <th scope="col">科目性質</th>
                  <th scope="col">節點類型</th>
                  <th scope="col">狀態</th>
                  <th scope="col" class="amount-column">科目餘額（TWD）</th>
                </tr>
              </thead>
              <tbody id="accounts-body"></tbody>
            </table>
          </section>

          <footer class="table-footer">
            <span id="account-count" class="numeric">0 個科目</span>
            <span id="balance-note">餘額依已過帳分錄計算；未載入時顯示 —。</span>
          </footer>
          <section class="account-summary" aria-label="所選科目摘要">
            <button class="secondary-button summary-action" id="manage-account" type="button" disabled>維護此科目</button>
            <h3 id="summary-name">選擇科目以查看摘要</h3>
            <p id="summary-path">科目完整路徑會顯示於此</p>
            <dl><div><dt>科目性質</dt><dd id="summary-element">—</dd></div><div><dt>節點類型</dt><dd id="summary-kind">—</dd></div><div><dt>狀態</dt><dd id="summary-state">—</dd></div><div><dt>科目餘額（TWD）</dt><dd class="amount-column" id="summary-balance">—</dd></div></dl>
          </section>
          <div id="account-maintenance-host"></div>
        </div>
        </section>
        <section id="workspace-page" aria-label="帳務工作區" hidden></section>
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
const balanceNote = document.querySelector<HTMLElement>('#balance-note')!
const search = document.querySelector<HTMLInputElement>('#account-search')!
const elementFilter = document.querySelector<HTMLSelectElement>('#element-filter')!
const balanceDate = document.querySelector<HTMLInputElement>('#balance-date')!
const inactiveButton = document.querySelector<HTMLButtonElement>('#inactive-button')!
const bookSelect = document.querySelector<HTMLSelectElement>('#book-select')!
const openBook = document.querySelector<HTMLButtonElement>('#open-book')!
const createBook = document.querySelector<HTMLButtonElement>('#create-book')!
const closeBook = document.querySelector<HTMLButtonElement>('#close-book')!
const refreshBooks = document.querySelector<HTMLButtonElement>('#refresh-books')!
const sessionMessage = document.querySelector<HTMLElement>('#session-message')!
const currentBookLabel = document.querySelector<HTMLElement>('#current-book')!
const accountPage = document.querySelector<HTMLElement>('#account-page')!
const workspacePage = document.querySelector<HTMLElement>('#workspace-page')!
const maintenanceHost = document.querySelector<HTMLElement>('#account-maintenance-host')!
const addAccount = document.querySelector<HTMLButtonElement>('#add-account')!
const manageAccount = document.querySelector<HTMLButtonElement>('#manage-account')!
const navigation = [...document.querySelectorAll<HTMLButtonElement>('[data-view]')]
const compactLayout = window.matchMedia('(max-width: 900px), (pointer: coarse)')
const supportsNativeInert = 'inert' in HTMLElement.prototype

const elementLabels: Record<AccountingElement, string> = {
  asset: '資產',
  liability: '負債',
  equity: '權益',
  income: '收益',
  expense: '費損',
}

let accountTree: AccountNode[] = []
let hasSuccessfulAccountResponse = false
const accountRequests = new LatestRequestGate()
const collapsedIds = new Set<string>()
let balances = new Map<string, bigint>()
let selectedAccountId: string | null = null
let selectedBookId: string | null = null
let selectionId: string | null = null
let showInactive = false
let sessionBusy = false
const sessionRequests = new LatestRequestGate()
type WorkspaceView = 'accounts' | 'journal' | 'storage' | 'open-items' | 'payment-memos' | ReportView
type WorkspaceController = { dispose(): void; hasUnsavedChanges(): boolean }
let activeView: WorkspaceView = 'accounts'
let workspaceController: WorkspaceController | null = null
let accountMaintenance: ReturnType<typeof mountAccountMaintenance> | null = null
let journalController: ReturnType<typeof mountJournalUI> | null = null
let workspaceEpoch = 0
let pendingWrites = 0
let pageActive = true
const today = new Date()
balanceDate.value = `${today.getFullYear()}-${String(today.getMonth() + 1).padStart(2, '0')}-${String(today.getDate()).padStart(2, '0')}`

type FocusTarget =
  | { kind: 'account'; accountId: string }
  | { kind: 'selected-account'; accountId: string }
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
  if (row.account.id === selectedAccountId) tableRow.classList.add('is-selected')

  const nameCell = document.createElement('th')
  nameCell.scope = 'row'
  const nameLayout = document.createElement('div')
  nameLayout.className = 'account-cell'
  nameLayout.style.setProperty('--tree-depth', String(row.depth))

  if (row.hasChildren) {
    const disclosure = document.createElement('button')
    const revealingMatches = search.value.trim() !== '' || elementFilter.value !== '' || showInactive
    const isCollapsed = !revealingMatches && collapsedIds.has(row.account.id)
    disclosure.disabled = revealingMatches
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

  const select = document.createElement('button')
  select.type = 'button'
  select.className = 'account-select'
  select.dataset.selectAccountId = row.account.id
  select.setAttribute('aria-pressed', String(row.account.id === selectedAccountId))
  select.append(createAccountName(row.account.name))
  select.addEventListener('click', () => {
    selectedAccountId = row.account.id
    renderAccounts()
    accountsBody.querySelector<HTMLButtonElement>(`button[data-select-account-id="${row.account.id}"]`)?.focus()
  })
  nameLayout.append(select)
  nameCell.append(nameLayout)

  const elementCell = document.createElement('td')
  elementCell.textContent = elementLabels[row.account.element]
  elementCell.className = `element-tag-cell element-${row.account.element}`

  const kindCell = document.createElement('td')
  kindCell.textContent = row.account.kind === 'category' ? '分類' : '過帳科目'

  const balanceCell = document.createElement('td')
  balanceCell.className = 'amount-column amount-unavailable'
  balanceCell.textContent = formatBalance(balances.get(row.account.id))
  if (row.account.kind === 'category') balanceCell.title = '分類餘額包含所有下層科目'

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
  const view = accountView(accountTree, collapsedIds, {
    query: search.value, element: elementFilter.value as AccountingElement | '', inactive: showInactive,
  })
  const rows = view.rows
  if (selectedAccountId && !rows.some(row => row.account.id === selectedAccountId)) selectedAccountId = null
  accountCount.textContent = `${view.matchingIds.size} 個${showInactive ? '停用' : '啟用'}科目${rows.length > view.matchingIds.size ? '（含上層路徑）' : ''}`
  const fragment = document.createDocumentFragment()
  rows.forEach((row) => fragment.append(renderAccountRow(row)))
  accountsBody.textContent = ''
  accountsBody.append(fragment)
  if (rows.length === 0) renderMessageRow('沒有符合的科目', '請調整搜尋或篩選條件。', 'message-row empty-row')
  renderSummary(view.paths.get(selectedAccountId ?? '') ?? [])
}

function renderSummary(path: AccountNode[]): void {
  const account = path[path.length - 1]?.account
  document.querySelector('#summary-name')!.replaceChildren(account ? createAccountName(account.name) : document.createTextNode('選擇科目以查看摘要'))
  const pathContainer = document.querySelector('#summary-path')!
  pathContainer.replaceChildren()
  path.forEach((node, index) => {
    if (index) pathContainer.append(' › ')
    pathContainer.append(createAccountName(node.account.name))
  })
  if (!account) pathContainer.textContent = '科目完整路徑會顯示於此'
  document.querySelector('#summary-element')!.textContent = account ? elementLabels[account.element] : '—'
  document.querySelector('#summary-kind')!.textContent = account ? account.kind === 'category' ? '分類（含下層餘額）' : '過帳科目' : '—'
  document.querySelector('#summary-state')!.textContent = account ? account.isActive ? '啟用' : '停用' : '—'
  document.querySelector('#summary-balance')!.textContent = formatBalance(account ? balances.get(account.id) : undefined)
  manageAccount.disabled = !account || pendingWrites > 0 || sessionBusy
}

function currentSelection(): SelectionSnapshot | null {
  return pageActive && !sessionBusy && selectedBookId && selectionId ? { bookId: selectedBookId, selectionId } : null
}

function disposeWorkspace(): void {
  workspaceEpoch += 1
  workspaceController?.dispose()
  workspaceController = null; accountMaintenance = null; journalController = null
  maintenanceHost.replaceChildren(); workspacePage.replaceChildren()
}

function workspaceContext(selection: SelectionSnapshot) {
  const epoch = workspaceEpoch
  const current = () => epoch === workspaceEpoch ? currentSelection() : null
  const request = pinnedWorkspaceRequest(selection, current, requestJson, delta => {
      pendingWrites += delta
      updateSessionControls()
    })
  const download = async (path: string, signal: AbortSignal) => {
    if (signal.aborted) throw new DOMException('已取消要求', 'AbortError')
    if (!sameSnapshot(selection, current())) throw new ApiError('selection_changed')
    startNativeDownload(path, selection.selectionId)
  }
  return {
    getSelection: current,
    request,
    upload: (path: string, signal: AbortSignal, file: File) => request(path, signal, 'POST', file),
    download,
    downloadStorage: (operationId: string, signal: AbortSignal) => download(`/api/storage/operations/${operationId}/download`, signal),
    importStorageContainer: (file: File, requestCapability: string, destinationCapability: string, signal: AbortSignal) => request(`/api/storage/container-imports?request=${encodeURIComponent(requestCapability)}&destination=${encodeURIComponent(destinationCapability)}`, signal, 'POST', file),
    selectStorage: async (kind: string, signal: AbortSignal) => {
      const value = await request('/api/native/selections', signal, 'POST', { kind })
      if (!value || typeof value !== 'object' || !('capability_id' in value)) throw new Error('原生選擇回應不符')
      const id = (value as { capability_id: unknown }).capability_id
      if (id === null) return null
      if (typeof id !== 'string' || !/^[a-f0-9]{32}$/.test(id)) throw new Error('原生選擇識別不符')
      return { capabilityId: id }
    },
    onChanged: () => { if (current()) void loadAccounts() },
    onSelectionChanged: () => {
      if (epoch !== workspaceEpoch) return
      void syncSession()
    },
    createAccountName,
    onOpenPosted: (id: string) => {
      if (switchView('journal')) journalController?.openPosted(id)
    },
    onOpenDraft: (id: string) => {
      if (switchView('journal')) journalController?.openDraft(id)
    },
  }
}

function renderWorkspace(): void {
  disposeWorkspace()
  accountPage.hidden = activeView !== 'accounts'
  workspacePage.hidden = activeView === 'accounts'
  for (const item of navigation) {
    const selected = item.dataset.view === activeView
    item.classList.toggle('is-selected', selected)
    if (selected) item.setAttribute('aria-current', 'page')
    else item.removeAttribute('aria-current')
  }
  const selection = currentSelection()
  if (!selection) {
    if (activeView !== 'accounts') {
      const message = document.createElement('p'); message.className = 'workspace-empty'
      message.textContent = '請先明確選擇帳簿。'
      workspacePage.append(message)
    }
    return
  }
  const context = workspaceContext(selection)
  if (activeView === 'accounts') {
    accountMaintenance = mountAccountMaintenance(maintenanceHost, context)
    workspaceController = accountMaintenance
  } else if (activeView === 'journal') {
    journalController = mountJournalUI(workspacePage, context)
    workspaceController = journalController
  } else if (activeView === 'open-items') {
    workspaceController = mountOpenItemsUI(workspacePage, context)
  } else if (activeView === 'payment-memos') {
    workspaceController = mountPaymentMemosUI(workspacePage, context)
  } else if (activeView === 'storage') {
    workspaceController = mountStorageUI(workspacePage, context)
  } else {
    workspaceController = mountReportsUI(workspacePage, context, activeView)
  }
}

function approveLeavingWorkspace(): boolean {
  if (pendingWrites > 0) {
    sessionMessage.textContent = '變更仍在處理中，請等候結果後再切換。'
    return false
  }
  return !workspaceController?.hasUnsavedChanges() || window.confirm('尚有未儲存輸入。要捨棄並離開目前畫面嗎？')
}

function switchView(view: WorkspaceView): boolean {
  if (view === activeView) return true
  if (!approveLeavingWorkspace()) return false
  activeView = view
  renderWorkspace()
  if (compactLayout.matches) setSidebarOpen(false)
  return true
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
  const selected = activeElement.closest<HTMLButtonElement>('.account-select')
  if (selected?.dataset.selectAccountId) return { kind: 'selected-account', accountId: selected.dataset.selectAccountId }

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
  } else if (target.kind === 'selected-account') {
    accountsBody.querySelector<HTMLButtonElement>(`button[data-select-account-id="${target.accountId}"]`)?.focus()
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
  if (!selectedBookId || sessionBusy) return
  let focusTarget = readFocusTarget()
  const preserveContent = hasSuccessfulAccountResponse
  const controller = accountRequests.begin()
  let requestSucceeded = false
  setLoadingState(preserveContent)

  try {
    const bookId = selectedBookId
    const generation = selectionId!
    const asOf = balanceDate.value
    if (!/^\d{4}-\d{2}-\d{2}$/.test(asOf)) throw new Error('請選擇有效日期')
    const responseBody = await requestJson('/api/accounts/tree', controller.signal, 'GET', undefined, generation)
    if (controller.signal.aborted) return
    if (!accountRequests.isCurrent(controller)) return
    const nextAccountTree = parseAccountTree(responseBody)
    let nextBalances = new Map<string, bigint>()
    let balanceUnavailable = false
    try {
      nextBalances = accountBalances(await requestJson(`/api/reports/trial-balance/${asOf}`, controller.signal, 'GET', undefined, generation), nextAccountTree, asOf)
    } catch (error) {
      if (error instanceof ApiError && error.code === 'selection_changed') throw error
      balanceUnavailable = true
    }
    const session = parseSession(await requestJson('/api/session', controller.signal))
    if (controller.signal.aborted) return
    if (!accountRequests.isCurrent(controller)) return
    if (session.bookId !== bookId || session.selectionId !== generation) { void syncSession(); return }

    focusTarget = reconcileFocusTarget(focusTarget)
    // Keep parsing local until the latest-request check above. This is the safe
    // equivalent of `accountTree = parseAccountTree(await response.json())`.
    accountTree = nextAccountTree
    balances = nextBalances
    balanceNote.textContent = balanceUnavailable ? `${asOf} 餘額無法核對，顯示 —；請重新整理。` : `截至 ${asOf}；借／貸表示餘額方向，分類包含下層；信用額度不計入。`
    hasSuccessfulAccountResponse = true
    const totalAccountCount = pruneCollapsedAccounts(accountTree)
    const rows = flattenAccountTree(accountTree, collapsedIds)
    accountCount.textContent = `${totalAccountCount} 個科目`
    statusLine.className = 'status-line is-ready'
    statusMessage.textContent = accountTree.length === 0 ? '尚未建立科目' : '科目資料已更新'

    if (rows.length === 0) {
      selectedAccountId = null
      renderSummary([])
      renderMessageRow(
        '尚未建立科目資料',
        '目前帳簿尚無科目；不會自動建立示意科目。',
        'message-row empty-row',
      )
    } else {
      renderAccounts()
    }
    requestSucceeded = true
  } catch (error) {
    if (error instanceof Error && error.name === 'AbortError') return
    if (!accountRequests.isCurrent(controller)) return
    if (error instanceof ApiError && error.code === 'selection_changed') { void syncSession(); return }
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
search.addEventListener('input', () => { if (hasSuccessfulAccountResponse) renderAccounts() })
elementFilter.addEventListener('change', () => { if (hasSuccessfulAccountResponse) renderAccounts() })
inactiveButton.addEventListener('click', () => {
  showInactive = !showInactive
  inactiveButton.setAttribute('aria-pressed', String(showInactive))
  if (hasSuccessfulAccountResponse) renderAccounts()
})
balanceDate.addEventListener('change', () => {
  balances.clear()
  balanceNote.textContent = '正在核對新日期的餘額；未載入時顯示 —。'
  if (hasSuccessfulAccountResponse) renderAccounts()
  void loadAccounts()
})
document.querySelector('#reset-filter')!.addEventListener('click', () => {
  search.value = ''; elementFilter.value = ''; showInactive = false
  inactiveButton.setAttribute('aria-pressed', 'false')
  if (hasSuccessfulAccountResponse) renderAccounts()
})

function clearBookView(): void {
  disposeWorkspace()
  accountRequests.cancel()
  accountTree = []; balances.clear(); collapsedIds.clear(); selectedAccountId = null
  hasSuccessfulAccountResponse = false
  accountCount.textContent = '0 個科目'
  balanceNote.textContent = '餘額依已過帳分錄計算；未載入時顯示 —。'
  tableFrame.removeAttribute('aria-busy')
  renderSummary([])
  renderMessageRow('請選擇帳簿', '選擇帳簿並按「開啟帳簿」後讀取資料。', 'message-row empty-row')
  if (activeView !== 'accounts') {
    const message = document.createElement('p'); message.className = 'workspace-empty'
    message.textContent = '正在確認帳簿，暫停顯示資料。'; workspacePage.append(message)
  }
}

function updateSessionControls(): void {
  bookSelect.disabled = sessionBusy || pendingWrites > 0 || bookSelect.options.length < 2
  openBook.disabled = sessionBusy || pendingWrites > 0 || !bookSelect.value
  createBook.disabled = sessionBusy || pendingWrites > 0
  closeBook.disabled = sessionBusy || pendingWrites > 0 || selectedBookId === null
  refreshBooks.disabled = sessionBusy || pendingWrites > 0
  refreshButton.disabled = sessionBusy || selectedBookId === null
  addAccount.disabled = sessionBusy || pendingWrites > 0 || selectedBookId === null
  manageAccount.disabled = sessionBusy || pendingWrites > 0 || selectedAccountId === null
  navigation.forEach(item => { item.disabled = sessionBusy || pendingWrites > 0 || selectedBookId === null })
  currentBookLabel.textContent = selectedBookId ? `帳簿：${selectedBookId}` : '尚未選簿'
}

async function syncSession(transition?: { method: 'POST' | 'DELETE'; bookId?: string }): Promise<void> {
  if (sessionBusy) return
  const controller = sessionRequests.begin()
  sessionBusy = true
  clearBookView()
  updateSessionControls()
  statusMessage.textContent = '正在確認帳簿…'
  let transitionFailure: unknown = null
  try {
    if (transition) {
      try {
        await requestJson('/api/session/selection', controller.signal, transition.method,
          transition.method === 'POST' ? { book_id: transition.bookId } : undefined, selectionId ?? 'unselected')
      } catch (error) { transitionFailure = error }
    }
    // Reconcile even after an interrupted write: the server owns transitions.
    const session = parseSession(await requestJson('/api/session', controller.signal))
    const books = parseBooks(await requestJson('/api/session/books', controller.signal))
    if (!sessionRequests.isCurrent(controller)) return
    selectedBookId = session.bookId
    selectionId = session.selectionId
    bookSelect.replaceChildren(new Option('請選擇帳簿', ''))
    books.forEach(id => bookSelect.append(new Option(id, id)))
    bookSelect.value = selectedBookId ?? ''
    sessionMessage.textContent = transitionFailure ? apiGuidance(transitionFailure) : books.length === 0 ? '沒有可用帳簿；請先準備新的本機帳簿。' : selectedBookId ? '帳簿已開啟。切換須另行選擇。' : '請明確選擇帳簿，不會自動開啟。'
    statusMessage.textContent = selectedBookId ? '正在讀取科目…' : '尚未選擇帳簿'
  } catch (error) {
    if (!sessionRequests.isCurrent(controller)) return
    selectedBookId = null
    selectionId = null
    sessionMessage.textContent = apiGuidance(error)
    statusMessage.textContent = '無法確認帳簿，請更新清單後再試。'
  } finally {
    if (sessionRequests.finish(controller)) {
      sessionBusy = false
      updateSessionControls()
      renderWorkspace()
      if (selectedBookId) void loadAccounts()
    }
  }
}
bookSelect.addEventListener('change', updateSessionControls)
createBook.addEventListener('click', () => {
  if (!approveLeavingWorkspace() || sessionBusy) return
  if (!window.confirm('建立全新空白帳簿，不會套用預設科目或自動開啟。請在 macOS 面板選擇帳簿集合內的新名稱。')) return
  void (async () => {
    const controller = sessionRequests.begin()
    pendingWrites += 1
    sessionBusy = true; clearBookView(); updateSessionControls()
    let message = '建立結果尚未確認；請更新清單並核對目的地，不要重複建立。'
    try {
      const value = await requestJson('/api/native/books', controller.signal, 'POST', {}, selectionId ?? 'unselected')
      if (!value || typeof value !== 'object' || !('book_id' in value) || !('selection_required' in value) || value.selection_required !== true || !(value.book_id === null || validBookId(value.book_id))) throw new Error('建立帳簿回應不符')
      message = value.book_id === null ? '已取消建立帳簿。' : '空白帳簿已建立；請從清單明確選擇後開啟。'
    } catch (error) { if (error instanceof ApiError) message = apiGuidance(error) }
    finally {
      pendingWrites -= 1
      if (sessionRequests.finish(controller)) { sessionBusy = false; await syncSession(); sessionMessage.textContent = message }
    }
  })()
})
openBook.addEventListener('click', () => { if (bookSelect.value && approveLeavingWorkspace()) void syncSession({ method: 'POST', bookId: bookSelect.value }) })
closeBook.addEventListener('click', () => { if (approveLeavingWorkspace()) void syncSession({ method: 'DELETE' }) })
refreshBooks.addEventListener('click', () => { if (approveLeavingWorkspace()) void syncSession() })
navigation.forEach(item => item.addEventListener('click', () => { switchView(item.dataset.view as WorkspaceView) }))
addAccount.addEventListener('click', () => { accountMaintenance?.openCreate(); maintenanceHost.scrollIntoView({ block: 'start' }) })
manageAccount.addEventListener('click', () => { if (selectedAccountId) accountMaintenance?.selectAccount(selectedAccountId); maintenanceHost.scrollIntoView({ block: 'start' }) })
window.addEventListener('beforeunload', event => {
  if (pendingWrites > 0 || workspaceController?.hasUnsavedChanges()) { event.preventDefault(); event.returnValue = '' }
})
window.addEventListener('pagehide', () => { pageActive = false; accountRequests.cancel(); sessionRequests.cancel(); sessionBusy = false; clearBookView() })
window.addEventListener('pageshow', (event) => {
  pageActive = true
  if (event.persisted) void syncSession()
})
clearBookView()
void syncSession()
