import { flattenAccountTree, parseAccountTree } from './account-tree.ts'
import type { AccountRecord, AccountingElement, AccountKind } from './account-tree.ts'
import { ApiError, apiGuidance } from './session-client.ts'
import { accountFailureRequiresReload, accountPathLabels, changeAccountCommand, createAccountCommand, deleteAccountCommand, elementLabels, eligibleParents, sameSelection, validateAccountCommandResponse } from './account-maintenance-model.ts'
import type { AccountChange, AccountCommand } from './account-maintenance-model.ts'

export interface AccountMaintenanceContext {
  getSelection(): { bookId: string; selectionId: string } | null
  request(path: string, signal: AbortSignal, method?: string, payload?: unknown): Promise<unknown>
  onChanged(): void
  onSelectionChanged(): void
  createAccountName(name: string): HTMLElement
}
export interface AccountMaintenanceController {
  dispose(): void
  hasUnsavedChanges(): boolean
  openCreate(): void
  selectAccount(id: string): void
}
const messages: Record<string, string> = {
  account_revision_conflict: '科目版本已改變。輸入已保留；請記下修改內容，再按重新載入後重新檢視。',
  duplicate_account_name: '同一層已有相同名稱，請修改名稱。',
  invalid_account_name: '名稱格式不符，請檢查名稱。',
  account_not_found: '科目已不存在，請重新載入科目。',
  account_rule_violation: '此操作不符合科目規則。請檢查子科目、啟用狀態、分錄引用與已過帳歷史；歷史資料不會被刪除。',
}
function element<K extends keyof HTMLElementTagNameMap>(tag: K, text?: string): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag)
  if (text !== undefined) node.textContent = text
  return node
}
function option(value: string, text: string): HTMLOptionElement {
  const node = element('option', text); node.value = value; return node
}
function field(label: string, input: HTMLElement): HTMLLabelElement {
  const node = element('label'); node.append(document.createTextNode(label), input); return node
}

export function mountAccountMaintenance(host: HTMLElement, context: AccountMaintenanceContext): AccountMaintenanceController {
  const selection = context.getSelection()
  const controller = new AbortController()
  let disposed = false, busy = false, dirty = false, needsReload = false
  let initialLoading = true, writing = false, queuedAccountId: string | undefined
  let accounts: AccountRecord[] = [], current: AccountRecord | undefined
  const section = element('section'); section.className = 'account-maintenance workspace-panel'
  section.setAttribute('aria-label', '科目維護')
  const toolbar = element('div'); toolbar.className = 'workspace-toolbar'
  const heading = element('h2', '科目維護')
  const picker = element('select'); picker.setAttribute('aria-label', '選擇維護科目')
  const create = element('button', '新增科目'); create.type = 'button'
  const reload = element('button', '重新載入'); reload.type = 'button'
  toolbar.append(heading, picker, create, reload)
  const status = element('p', '載入科目中…'); status.setAttribute('role', 'status')
  const editor = element('div'); editor.className = 'maintenance-editor'
  section.append(toolbar, status, editor); host.append(section)

  function alive(): boolean {
    if (disposed) return false
    if (!sameSelection(selection, context.getSelection())) {
      controller.abort(); section.inert = true; status.textContent = '帳簿選擇已變更，請重新開啟科目維護。'; context.onSelectionChanged(); return false
    }
    return true
  }
  function lock(value: boolean): void {
    busy = value; picker.disabled = value; create.disabled = value
    for (const button of editor.querySelectorAll('button')) button.disabled = value || needsReload
    for (const input of editor.querySelectorAll<HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement>('input, select, textarea')) input.disabled = value
    reload.disabled = value
  }
  function report(error: unknown, mutation = false): void {
    if (!alive()) return
    if (error instanceof ApiError && error.code === 'selection_changed') {
      needsReload = true; lock(false); context.onSelectionChanged(); return
    }
    if (mutation && accountFailureRequiresReload(error instanceof ApiError ? error : null)) needsReload = true
    status.textContent = error instanceof ApiError ? messages[error.code] ?? apiGuidance(error) : '無法安全完成要求；輸入已保留，請檢查服務後重新載入。'
    if (mutation && needsReload && !(error instanceof ApiError && messages[error.code])) status.textContent = '未能確認操作結果，可能已儲存。輸入已保留；請先重新載入並確認科目，避免重複建立。'
    lock(false)
  }
  async function execute(command: AccountCommand): Promise<void> {
    if (!alive() || busy || needsReload) return
    writing = true
    lock(true)
    try {
      const response = await context.request(command.path, controller.signal, command.method, command.payload)
      if (!alive()) return
      validateAccountCommandResponse(command, response, current, new Set(accounts.map(account => account.id)))
      dirty = false; current = undefined; context.onChanged()
      await load()
      if (alive() && !needsReload) status.textContent = '科目變更已儲存。'
    } catch (error) { report(error, true) }
    finally { writing = false; if (alive()) lock(false) }
  }
  function save(command: () => AccountCommand): void {
    try { void execute(command()) } catch (error) {
      status.textContent = error instanceof Error ? error.message : '請檢查輸入。'
    }
  }
  function action(text: string, handler: () => void): HTMLButtonElement {
    const button = element('button', text); button.type = 'button'; button.addEventListener('click', handler); return button
  }
  function render(account?: AccountRecord): void {
    current = account; editor.replaceChildren(); dirty = false
    const name = element('input'); name.type = 'text'; name.maxLength = 240; name.value = account?.name ?? ''; name.autocomplete = 'off'
    const preview = element('span'); preview.className = 'account-name-preview'
    function showName(): void { preview.replaceChildren(context.createAccountName(name.value)) }
    name.addEventListener('input', showName); showName()
    const parent = element('select')
    parent.append(option('', '最上層'))
    const paths = accountPathLabels(accounts)
    for (const item of eligibleParents(accounts, account)) parent.append(option(item.id, paths.get(item.id)!))
    parent.value = account?.parentId ?? ''
    const kind = element('select'); kind.append(option('posting', '過帳科目'), option('category', '分類科目')); kind.value = account?.kind ?? 'posting'
    const category = element('select')
    for (const [value, label] of Object.entries(elementLabels)) category.append(option(value, label))
    category.value = account?.element ?? 'asset'
    const form = element('div'); form.className = 'maintenance-fields'
    form.append(field('科目名稱', name), preview, field('上層科目', parent), field('科目用途', kind), field('科目性質', category))
    for (const input of [name, parent, kind, category]) {
      input.addEventListener('input', () => { dirty = true })
      input.addEventListener('change', () => { dirty = true })
    }
    if (!account) {
      parent.addEventListener('change', () => {
        const chosen = accounts.find(item => item.id === parent.value)
        if (chosen) category.value = chosen.element
      })
      category.addEventListener('change', () => {
        const chosen = accounts.find(item => item.id === parent.value)
        if (chosen && chosen.element !== category.value) parent.value = ''
      })
      form.append(action('建立科目', () => save(() => createAccountCommand(name.value, category.value as AccountingElement, kind.value as AccountKind, parent.value || null))))
    } else {
      const title = element('p'); title.append(context.createAccountName(account.name), document.createTextNode(` · ${account.isActive ? '啟用中' : '已停用'}`))
      editor.append(title)
      function change(value: AccountChange): void {
        const pendingOtherChanges = (value.operation !== 'rename' && name.value !== account!.name) ||
          (value.operation !== 'move' && (parent.value || null) !== account!.parentId) ||
          (value.operation !== 'change_kind' && kind.value !== account!.kind) ||
          (value.operation !== 'change_element' && category.value !== account!.element)
        if (pendingOtherChanges && !window.confirm('其他欄位也有未儲存輸入。此次只儲存選定操作，並捨棄其他輸入，是否繼續？')) return
        save(() => changeAccountCommand(account!, value))
      }
      form.append(
        action('儲存名稱', () => change({ operation: 'rename', name: name.value })),
        action('移動科目', () => change({ operation: 'move', parent_id: parent.value || null })),
        action('變更用途', () => change({ operation: 'change_kind', kind: kind.value as AccountKind })),
        action('變更科目性質', () => change({ operation: 'change_element', element: category.value as AccountingElement })),
        action(account.isActive ? '停用科目' : '啟用科目', () => change({ operation: 'set_active', is_active: !account.isActive })),
      )
      const confirm = element('input'); confirm.type = 'checkbox'
      const explanation = field('確認刪除未被分錄引用的末層科目', confirm)
      const remove = action('刪除科目', () => {
        if (!confirm.checked) { status.textContent = '請先勾選刪除確認。'; return }
        save(() => deleteAccountCommand(account!))
      })
      form.append(explanation, remove)
      editor.append(element('p', '每次只儲存一項變更。變更後重新讀取科目；有歷史、分錄引用或子科目的限制由後端原子檢查。'))
    }
    editor.append(form); lock(busy)
  }
  function discard(): boolean {
    if (needsReload) return window.confirm('尚有未確認的科目操作結果。請保留輸入，並重新載入確認；要捨棄目前顯示內容並繼續嗎？')
    return !dirty || window.confirm('尚有未儲存輸入。要捨棄並繼續嗎？')
  }
  async function load(): Promise<void> {
    if (!alive()) return
    lock(true)
    try {
      const value = await context.request('/api/accounts/tree', controller.signal)
      if (!alive()) return
      accounts = flattenAccountTree(parseAccountTree(value), new Set()).map(row => row.account)
      const paths = accountPathLabels(accounts)
      picker.replaceChildren(option('', '新增科目'))
      for (const account of accounts) picker.append(option(account.id, `${paths.get(account.id)!}${account.isActive ? '' : '（已停用）'}`))
      needsReload = false; render(); status.textContent = `共 ${accounts.length} 個科目。選擇科目以維護，或建立新科目。`
    } catch (error) { needsReload = true; report(error) }
    finally {
      initialLoading = false
      if (alive()) {
        lock(false)
        if (queuedAccountId !== undefined && !needsReload) {
          const id = queuedAccountId; queuedAccountId = undefined; selectAccount(id)
        }
      }
    }
  }
  function openCreate(): void {
    if (!alive() || busy || !discard()) return
    picker.value = ''; render(); status.textContent = '新增科目；不預設範例分類。'
  }
  function selectAccount(id: string): void {
    if (initialLoading && busy && !writing && alive()) { queuedAccountId = id; return }
    if (!alive() || busy || !discard()) { picker.value = current?.id ?? ''; return }
    const account = accounts.find(item => item.id === id)
    if (!account) return
    picker.value = id; render(account); status.textContent = '選擇要執行的個別維護操作。'
  }
  create.addEventListener('click', openCreate)
  picker.addEventListener('change', () => { if (picker.value) selectAccount(picker.value); else openCreate() })
  reload.addEventListener('click', () => { if (!busy && discard()) void load() })
  if (selection) void load(); else status.textContent = '請先選擇帳簿。'
  return { dispose() { disposed = true; controller.abort(); section.remove() }, hasUnsavedChanges: () => dirty || writing || needsReload, openCreate, selectAccount }
}
