import { attachDatePicker } from './date-picker.ts'
import { parseAccountTree } from './account-tree.ts'
import { ApiError, apiGuidance } from './session-client.ts'
import { amount, parseDraftList, reversalDocument, verifyJournalResult, draftSaveCommand, journalDocument, parseJournalEntry, postingAccounts, validDate, normalizeTime, type JournalEntry, type JournalInput } from './journal-model.ts'
import { mountAttachmentsUI } from './attachments-ui.ts'
import type { AttachmentMetadata } from './attachments-model.ts'

interface JournalContext {
  getSelection(): { bookId: string; selectionId: string } | null
  request(path: string, signal: AbortSignal, method?: string, payload?: unknown): Promise<unknown>
  upload?(path: string, signal: AbortSignal, file: File): Promise<unknown>
  download?(path: string, signal: AbortSignal, metadata: AttachmentMetadata): Promise<void>
  onChanged(): void
  onSelectionChanged(): void
  createAccountName(name: string): HTMLElement
}
function el<K extends keyof HTMLElementTagNameMap>(tag: K, text = '', className = ''): HTMLElementTagNameMap[K] { const node = document.createElement(tag); node.textContent = text; node.className = className; return node }
function today(): string { const now = new Date(); return `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, '0')}-${String(now.getDate()).padStart(2, '0')}` }
function use24HourTime(node: HTMLInputElement): void {
  node.type = 'text'; node.placeholder = 'HH:mm:ss'; node.maxLength = 8
  node.className = 'journal-time-input'
  node.setAttribute('pattern', '(?:[01][0-9]|2[0-3]):[0-5][0-9]:[0-5][0-9]')
  node.setAttribute('title', '24 小時制，例如 14:35:27；可留白')
  node.setAttribute('autocomplete', 'off'); node.setAttribute('spellcheck', 'false')
}
function empty(): JournalInput { return { date: today(), time: '', reference: '', explanation: '', lines: [{ accountId: '', debit: '', credit: '', memo: '' }, { accountId: '', debit: '', credit: '', memo: '' }] } }
export function mountJournalUI(host: HTMLElement, context: JournalContext): { dispose(): void; hasUnsavedChanges(): boolean; openPosted(id: string): void; openDraft(id: string): void } {
  const controller = new AbortController(); const initialSelection = context.getSelection()?.selectionId
  let disposed = false; let busy = false; let dirty = false; let uncertainWrite = false; let writeAttempted = false; let attachmentBusy = false
  let current: JournalEntry | null = null; let input = empty(); let correctionId: string | null = null
  let ready = false; let pendingPosted: string | null = null; let pendingDraft: string | null = null
  let accounts: { id: string; label: string }[] = []; let allNames = new Map<string, string>(); let drafts: JournalEntry[] = []
  const root = el('section', '', 'journal-ui'); const message = el('p', '正在讀取草稿與科目…', 'journal-message status-line'); message.setAttribute('role', 'status'); message.setAttribute('aria-live', 'polite')
  const toolbar = el('div', '', 'journal-toolbar header-actions'); const list = el('div', '', 'journal-draft-list'); const editor = el('div', '', 'journal-editor'); const confirmation = el('div', '', 'journal-confirmation')
  const attachmentHost = el('div')
  root.append(el('h2', '日記簿'), toolbar, message, list, editor, confirmation, attachmentHost); host.replaceChildren(root)
  const attachments = mountAttachmentsUI(attachmentHost, {
    request: context.request,
    ...(context.upload ? { upload: context.upload } : {}), ...(context.download ? { download: context.download } : {}),
    onSelectionChanged: context.onSelectionChanged,
    onBusy(value) { attachmentBusy = value; if (usable()) render() },
    async onEntryChanged() {
      if (!current || !usable()) throw new Error('請重新確認目前分錄。')
      if (current.revision !== null) {
        const target = current; const next = parseJournalEntry(await request(`/api/journal/drafts/${target.id}`, controller.signal), true)
        if (next.id !== target.id || JSON.stringify(next.input) !== JSON.stringify(target.input) || BigInt(next.revision!) < BigInt(target.revision!)) throw new Error('分錄內容或版本已改變；請先重新載入分錄。')
        if (!usable()) return; current = next; input = next.input
      }
      context.onChanged(); render()
    },
  })
  function usable(): boolean { return !disposed && initialSelection !== undefined && context.getSelection()?.selectionId === initialSelection }
  async function request(path: string, signal: AbortSignal, method = 'GET', payload?: unknown): Promise<unknown> {
    if (method !== 'GET') {
      if (uncertainWrite) throw new Error('先前寫入結果或版本尚未核對；請重新載入分錄並核對日記簿後再操作。')
      writeAttempted = true
    }
    return context.request(path, signal, method, payload)
  }
  function button(label: string, run: () => void, primary = false): HTMLButtonElement { const node = el('button', label, primary ? 'primary-button' : 'secondary-button'); node.type = 'button'; node.disabled = busy || attachmentBusy; node.addEventListener('click', run); return node }
  function fail(error: unknown): void {
    if (!usable()) return
    if (error instanceof ApiError && error.code === 'selection_changed') { context.onSelectionChanged(); return }
    const code = error instanceof ApiError ? error.code : ''
    message.textContent = code === 'stale_draft' || code.includes('revision') ? '草稿已有較新版本；你的輸入已保留，請先記下變更再重新載入。' : error instanceof ApiError ? `${apiGuidance(error)} 輸入已保留；寫入不會自動重試。` : error instanceof Error ? error.message : '無法完成要求；輸入已保留。'
  }
  function requiresReconciliation(error: unknown): boolean {
    if (!(error instanceof ApiError)) return true
    const status = (error as ApiError & { status?: number | null }).status
    if (status === 409 || ['stale_draft', 'draft_revision_conflict', 'journal_state_conflict', 'selection_changed'].includes(error.code)) return true
    return !(typeof status === 'number' && status >= 400 && status < 500)
  }
  async function run(action: () => Promise<void>, write = false): Promise<void> {
    if (busy || attachmentBusy || !usable()) return
    if (write && attachments.hasUnsavedChanges()) { message.textContent = '請先完成或取消附件選擇，並核對附件操作結果。'; return }
    busy = true; writeAttempted = false; confirmation.replaceChildren(); render()
    try { await action() } catch (error) { if (write && writeAttempted && requiresReconciliation(error)) uncertainWrite = true; fail(error) } finally { busy = false; if (usable()) { render(); if (ready && pendingPosted) { const id = pendingPosted; pendingPosted = null; openPosted(id) } else if (ready && pendingDraft) { const id = pendingDraft; pendingDraft = null; openDraft(id) } } }
  }
  function openDraft(id: string): void {
    if (!/^[1-9]\d*$/.test(id) || BigInt(id) > 9223372036854775807n) { message.textContent = '請選擇有效草稿。'; return }
    pendingPosted = null
    if (busy || !ready) { pendingDraft = id; return }
    discard(() => void run(async () => {
      const next = parseJournalEntry(await request(`/api/journal/drafts/${id}`, controller.signal), true)
      if (next.id !== id) throw new Error('草稿回應識別不符。')
      if (!usable()) return
      current = next; input = next.input; correctionId = null; dirty = false; uncertainWrite = false
      message.textContent = '已開啟草稿；請核對內容，明確確認過帳後才會影響帳務。'
    }))
  }
  function openPosted(id: string): void {
    if (!/^[1-9]\d*$/.test(id) || BigInt(id) > 9223372036854775807n) { message.textContent = '請選擇有效分錄。'; return }
    pendingDraft = null
    if (busy || !ready) { pendingPosted = id; return }
    discard(() => void run(async () => { const next = parseJournalEntry(await request(`/api/journal/posted/${id}`, controller.signal), false); if (next.id !== id) throw new Error('分錄回應識別不符。'); if (!usable()) return; current = next; input = next.input; correctionId = null; dirty = false; uncertainWrite = false; message.textContent = '已過帳分錄不可修改；可另建沖銷或更正分錄。' }))
  }
  function discard(action: () => void): void { if ((!dirty && !uncertainWrite && !attachments.hasUnsavedChanges()) || window.confirm('捨棄尚未儲存輸入、附件選擇或尚未核對的寫入結果，並繼續？')) action() }
  function ask(label: string, action: () => Promise<void>, confirmLabel = '確認'): void {
    if (uncertainWrite) { message.textContent = '先前寫入結果或版本尚未核對；請重新載入分錄並核對日記簿後再操作。'; return }
    confirmation.replaceChildren(el('p', label), button(confirmLabel, () => void run(action, true), true), button('取消', () => confirmation.replaceChildren()))
  }
  function field(label: string, value: string, update: (value: string) => void, type = 'text'): HTMLLabelElement {
    const wrapper = el('label', label); const node = el('input'); node.type = type; if (type === 'time') use24HourTime(node); node.value = value; node.disabled = busy || attachmentBusy || (current?.revision === null && correctionId === null)
    node.addEventListener('input', () => { update(node.value); dirty = true; confirmation.replaceChildren(); updateSummary(); syncAttachmentState() }); wrapper.append(node); if (type === 'date') attachDatePicker(node); return wrapper
  }
  const summary = el('footer', '', 'journal-summary table-footer')
  const summaryText = el('span', '', 'journal-summary-text')
  function updateSummary(): void { let debit = 0n; let credit = 0n; let invalid = false; input.lines.forEach(line => { try { if (line.debit) debit += amount(line.debit); if (line.credit) credit += amount(line.credit) } catch { invalid = true } }); summaryText.textContent = `借方 ${debit.toLocaleString('en-US')}　貸方 ${credit.toLocaleString('en-US')}　差額 ${debit >= credit ? '借' : '貸'} ${(debit >= credit ? debit - credit : credit - debit).toLocaleString('en-US')} TWD${invalid ? '；有無效金額' : ''}${dirty ? '　尚未儲存' : ''}` }
  function syncAttachmentState(): void { attachments.setEntry(correctionId ? null : current ? { id: current.id, revision: current.revision } : null, busy || dirty || uncertainWrite || correctionId !== null) }
  function render(): void {
    confirmation.replaceChildren()
    const readonly = current?.revision === null && correctionId === null
    toolbar.replaceChildren(button('新增草稿', () => discard(() => { current = null; input = empty(); correctionId = null; dirty = false; uncertainWrite = false; render() })), button('更新草稿與科目', () => void run(async () => { await loadAccounts(); await loadDrafts(); ready = true; message.textContent = '科目選項與草稿清單已更新；編輯內容保留。' })))
    list.replaceChildren(el('h3', `草稿（${drafts.length}）`))
    drafts.forEach(entry => list.append(button(`${entry.input.date}　${entry.input.reference || '未填參考編號'}　${entry.input.explanation}`, () => discard(() => void run(async () => { const next = parseJournalEntry(await request(`/api/journal/drafts/${entry.id}`, controller.signal), true); if (next.id !== entry.id) throw new Error('草稿回應識別不符。'); if (!usable()) return; current = next; input = next.input; correctionId = null; dirty = false; uncertainWrite = false })))) )
    const lookup = el('input'); lookup.placeholder = '已過帳分錄識別'; lookup.setAttribute('aria-label', '已過帳分錄識別'); lookup.inputMode = 'numeric'; lookup.disabled = busy
    toolbar.append(lookup, button('檢視已過帳', () => { if (!/^[1-9]\d*$/.test(lookup.value)) { message.textContent = '請輸入有效分錄識別。'; return }; const lookupId = lookup.value; discard(() => void run(async () => { const next = parseJournalEntry(await request(`/api/journal/posted/${lookupId}`, controller.signal), false); if (next.id !== lookupId) throw new Error('分錄回應識別不符。'); if (!usable()) return; current = next; input = next.input; correctionId = null; dirty = false; uncertainWrite = false })) }))
    const editorHeader = el('div', '', 'journal-editor-heading')
    editorHeader.append(el('h3', correctionId ? '建立連結更正分錄' : readonly ? '已過帳分錄（不可修改）' : current ? '編輯草稿' : '新增草稿'))
    editor.replaceChildren(editorHeader)
    if (current) editor.append(el('p', `分錄 ${current.id}${current.revision ? `・草稿版本 ${current.revision}` : `・${current.relationship}${current.originalId ? `・原分錄 ${current.originalId}` : ''}`}`))
    if (correctionId) editor.append(el('p', `更正原分錄 ${correctionId}；須填寫說明，確認後直接建立新的已過帳分錄。`))
    const displayDate = (): string => input.date + (input.time ? `\n${input.time}` : '')
    const syncDateCell = (): void => { const dateCell = editor.querySelector('.journal-entry-date'); if (dateCell) dateCell.textContent = displayDate() }
    const header = el('div', '', 'journal-header-fields filter-bar')
    header.append(field('會計日期', input.date, value => { input.date = value; syncDateCell() }, 'date'), field('時間（選填）', input.time ?? '', value => { input.time = /^(?:[01]\d|2[0-3]):[0-5]\d(?::[0-5]\d)?$/.test(value) ? normalizeTime(value) ?? '' : value; syncDateCell() }, 'time'), field('參考編號', input.reference, value => { input.reference = value }))
    editor.append(header)
    const table = el('table', '', 'journal-lines'); const head = el('thead'); const headings = el('tr'); ['日期', '科目／摘要', '借方 TWD', '貸方 TWD', ''].forEach(label => headings.append(el('th', label))); head.append(headings); table.append(head); const body = el('tbody')
    input.lines.forEach((line, index) => {
      const row = el('tr'); row.append(el('td', index === 0 ? displayDate() : '', 'journal-entry-date')); const cell = el('td'); const indent = () => cell.className = Boolean(line.credit) && !line.debit ? 'journal-credit-account' : ''; indent(); const accountInput = el('input'); accountInput.type = 'search'; accountInput.value = line.accountSearch ?? allNames.get(line.accountId) ?? ''; accountInput.setAttribute('aria-label', `第 ${index + 1} 列科目名稱與路徑`); accountInput.disabled = busy || readonly; const datalist = el('datalist'); datalist.id = `journal-accounts-${index}`; accountInput.setAttribute('list', datalist.id)
      accounts.forEach(account => { const option = el('option'); option.value = account.label; datalist.append(option) }); accountInput.addEventListener('input', () => { line.accountSearch = accountInput.value; confirmation.replaceChildren(); const matches = accounts.filter(account => account.label === accountInput.value); line.accountId = matches.length === 1 ? matches[0]!.id : ''; dirty = true; updateSummary(); syncAttachmentState() }); const account = el('div', '', 'journal-account-with-note')

      account.append(readonly ? context.createAccountName(allNames.get(line.accountId) ?? '科目資料不可用') : accountInput, datalist)
      if (!readonly || line.memo) {
        const note = el('details', '', `journal-note${line.memo ? ' has-note' : ''}`)
        const trigger = el('summary'); trigger.setAttribute('aria-label', `第 ${index + 1} 列${readonly ? '檢視' : '編輯'}註解`)
        trigger.title = line.memo ? '查看科目註解' : '新增科目註解'
        const icon = el('span', '', 'journal-comment-icon'); icon.setAttribute('aria-hidden', 'true'); trigger.append(icon)
        const panel = el('div', '', 'journal-note-panel')
        if (readonly) panel.append(el('p', line.memo))
        else panel.append(field(`第 ${index + 1} 列摘要`, line.memo, value => {
          line.memo = value; note.className = `journal-note${value.trim() ? ' has-note' : ''}`
          trigger.title = value.trim() ? '查看科目註解' : '新增科目註解'
        }))
        note.addEventListener('keydown', event => { if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); note.open = false; trigger.focus() } })
        note.append(trigger, panel); account.append(note)
      }
      cell.append(account); row.append(cell)
      ;(['debit', 'credit'] as const).forEach(key => { const c = el('td'); c.className = 'amount-column'; const item = field(`第 ${index + 1} 列${key === 'debit' ? '借方' : '貸方'}`, line[key], value => { line[key] = value; indent() }); const control = item.querySelector('input')!; control.inputMode = 'numeric'; if (readonly) c.textContent = line[key] ? amount(line[key]).toLocaleString('en-US') : ''; else c.append(item); row.append(c) })
      const actions = el('td'); if (!readonly) actions.append(button('移除列', () => { input.lines.splice(index, 1); dirty = true; render() })); row.append(actions); body.append(row)
    }); const narrative = el('tr', '', 'journal-narrative'); narrative.append(el('td')); const narrativeCell = el('td'); narrativeCell.colSpan = 3; if (readonly) narrativeCell.append(el('p', input.explanation ? `（${input.explanation}）` : ''))
    else {
      const narrativeField = field('說明', input.explanation, value => { input.explanation = value })
      const narrativeInput = narrativeField.querySelector('input')!; narrativeInput.setAttribute('aria-label', '分錄說明'); narrativeInput.placeholder = '輸入說明（選填）'
      narrativeField.replaceChildren(narrativeInput); narrativeCell.append(narrativeField)
    }; narrative.append(narrativeCell, el('td')); body.append(narrative); table.append(body); const frame = el('div', '', 'table-frame'); frame.append(table); summary.replaceChildren(summaryText); editor.append(frame, summary); updateSummary()
    if (!readonly) {
      const rowTools = el('div', '', 'journal-row-tools')
      rowTools.append(button('新增列', () => { input.lines.push({ accountId: '', debit: '', credit: '', memo: '' }); dirty = true; render() }))
      summary.append(rowTools)
    }
    const actionBar = el('div', '', 'journal-workflow-actions')
    const maintenanceActions = el('div', '', 'journal-maintenance-actions')
    const actions = el('div', '', 'journal-toolbar header-actions journal-commit-actions')
    actionBar.append(actions, maintenanceActions)
    if (!readonly) {
      if (!correctionId) actions.append(button('儲存草稿', () => void run(async () => { if (uncertainWrite) throw new Error('先前寫入結果未確認；請先重新載入草稿或清單核對，避免重複寫入。'); const expected = journalDocument(input, false, new Set(accounts.map(a => a.id))); const command = draftSaveCommand(current, expected); const next = parseJournalEntry(await request(command.path, controller.signal, command.method, command.payload), true); verifyJournalResult(next, expected, current ? 'replace' : 'create', current ?? undefined); if (!usable()) return; current = next; input = next.input; dirty = false; uncertainWrite = false; context.onChanged(); message.textContent = '草稿已明確儲存。'; await refreshAfterWrite() }, true), true))
      actions.append(button(correctionId ? '確認建立更正分錄' : '過帳已儲存草稿', () => { try { journalDocument(input, true, new Set(accounts.map(a => a.id))); if (uncertainWrite) throw new Error('請先核對前次寫入結果。'); if (!correctionId && (!current || dirty)) throw new Error('請先明確儲存草稿，再確認過帳。'); if (correctionId && !input.explanation.trim()) throw new Error('更正分錄須填寫說明。'); ask('確認過帳？過帳後不能修改或刪除；借貸平衡與科目狀態由後端原子驗證。', async () => { const target = current!; const expected = journalDocument(input, true, new Set(accounts.map(a => a.id))); const wasCorrection = correctionId !== null; const result = correctionId ? await request(`/api/journal/posted/${correctionId}/correction`, controller.signal, 'POST', journalDocument(input, true, new Set(accounts.map(a => a.id)))) : await request(`/api/journal/drafts/${current!.id}/post`, controller.signal, 'POST', { expected_revision: current!.revision }); const next = parseJournalEntry(result, false); verifyJournalResult(next, expected, wasCorrection ? 'correction' : 'post', target); if (!usable()) return; current = next; input = next.input; correctionId = null; dirty = false; uncertainWrite = false; context.onChanged(); message.textContent = '分錄已過帳。'; await refreshAfterWrite() }) } catch (error) { fail(error) } }))
      if (current?.revision && !correctionId) {
        const deleteDraft = button('刪除草稿', () => {
          if (uncertainWrite) { message.textContent = '請先核對前次寫入結果。'; return }
          const target = current!
          confirmation.replaceChildren(el('p', `要刪除草稿 ${target.id} 嗎？目前內容將被移除。`), button('繼續刪除', () => {
            ask('最後確認：確定刪除這份草稿？刪除後無法復原。', async () => {
              const result = await request(`/api/journal/drafts/${target.id}`, controller.signal, 'DELETE', { expected_revision: target.revision })
              if (result !== null) throw new Error('刪除回應格式不符；請重新載入核對。')
              if (!usable()) return
              current = null; input = empty(); dirty = false; uncertainWrite = false; context.onChanged(); await refreshAfterWrite()
            }, '確認刪除')
          }), button('取消', () => confirmation.replaceChildren()))
        })
        deleteDraft.className = 'secondary-button journal-delete-button'; deleteDraft.setAttribute('aria-label', '刪除草稿'); deleteDraft.title = '刪除草稿'
        maintenanceActions.append(deleteDraft)
      }
    } else if (current) {
      actions.append(button('建立更正分錄', () => { correctionId = current!.id; input = { ...input, explanation: '', lines: input.lines.map(line => ({ ...line })) }; dirty = true; render() }))
      if (current.relationship !== 'reversal') { const reversalDate = el('input'); reversalDate.type = 'date'; reversalDate.value = today() < current.input.date ? current.input.date : today(); reversalDate.min = current.input.date; reversalDate.setAttribute('aria-label', '沖銷日期'); reversalDate.disabled = busy; reversalDate.addEventListener('input', () => confirmation.replaceChildren()); const reversalTime = el('input'); use24HourTime(reversalTime); reversalTime.setAttribute('aria-label', '沖銷時間（選填）'); reversalTime.disabled = busy; reversalTime.addEventListener('input', () => confirmation.replaceChildren()); actions.append(reversalDate, reversalTime, button('沖銷分錄', () => { const original = current!; const date = reversalDate.value; if (!validDate(date) || date < original.input.date) { message.textContent = '沖銷日期須不早於原分錄日期。'; return }; let time: string | null; try { time = normalizeTime(reversalTime.value) } catch (error) { fail(error); return }; ask('確認建立借貸互換的沖銷分錄？停用科目須先重新啟用；每筆原分錄最多一次直接沖銷。', async () => { const next = parseJournalEntry(await request(`/api/journal/posted/${original.id}/reversal`, controller.signal, 'POST', { entry_date: date, entry_time: time }), false); verifyJournalResult(next, reversalDocument(original, date, time ?? ''), 'reversal', original); if (!usable()) return; current = next; input = next.input; dirty = false; uncertainWrite = false; context.onChanged(); message.textContent = '已建立沖銷分錄。' }) })) }
    }
    editorHeader.append(actionBar, confirmation)
    if (readonly) { const reversalDate = actions.querySelector<HTMLInputElement>('input[type="date"]'); if (reversalDate) attachDatePicker(reversalDate) }
    syncAttachmentState()
  }
  async function refreshAfterWrite(): Promise<void> { writeAttempted = false; try { await loadDrafts() } catch (error) { fail(error) } }
  async function loadDrafts(): Promise<void> { const parsed = parseDraftList(await request('/api/journal/drafts', controller.signal)); if (usable()) drafts = parsed }
  async function loadAccounts(): Promise<void> {
    const tree = parseAccountTree(await request('/api/accounts/tree', controller.signal)); const names = new Map<string, string>()
    function walk(nodes: typeof tree, path: string[]): void { nodes.forEach(node => { const next = [...path, node.account.name]; names.set(node.account.id, next.join(' › ')); walk(node.children, next) }) }
    walk(tree, []); if (!usable()) return; accounts = postingAccounts(tree); allNames = names
  }
  void run(async () => { await loadAccounts(); await loadDrafts(); if (!usable()) return; ready = true; message.textContent = '草稿須明確儲存；金額為正整數新臺幣。' })
  return { dispose() { disposed = true; controller.abort(); attachments.dispose(); root.remove() }, hasUnsavedChanges() { return dirty || busy || uncertainWrite || attachments.hasUnsavedChanges() }, openPosted, openDraft }
}
