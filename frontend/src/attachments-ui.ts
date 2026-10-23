import { ApiError, apiGuidance } from './session-client.ts'
import { detachCommand, parseAttachment, parseAttachments, uploadPath, verifyAttachmentResult, type AttachmentEntry, type AttachmentMetadata } from './attachments-model.ts'
export interface AttachmentContext {
  request(path: string, signal: AbortSignal, method?: string, payload?: unknown): Promise<unknown>
  upload?(path: string, signal: AbortSignal, file: File): Promise<unknown>
  download?(path: string, signal: AbortSignal, metadata: AttachmentMetadata): Promise<void>
  onEntryChanged(): Promise<void>
  onBusy(busy: boolean): void
  onSelectionChanged(): void
}
function node<K extends keyof HTMLElementTagNameMap>(tag: K, text = ''): HTMLElementTagNameMap[K] { const result = document.createElement(tag); result.textContent = text; return result }
export function mountAttachmentsUI(host: HTMLElement, context: AttachmentContext): { setEntry(entry: AttachmentEntry | null, blocked: boolean): void; hasUnsavedChanges(): boolean; dispose(): void } {
  let entry: AttachmentEntry | null = null; let blocked = true; let busy = false; let disposed = false; let uncertain = false; let selected: File | null = null
  let items: AttachmentMetadata[] = []; let verified = new Set<string>(); let generation = 0; let requestController = new AbortController()
  const root = node('section'); root.className = 'attachments-ui'; root.setAttribute('aria-label', '分錄附件')
  const heading = node('h3', '分錄附件'); const status = node('p', '先儲存分錄，再新增附件。'); status.className = 'attachments-status'; status.setAttribute('role', 'status'); status.setAttribute('aria-live', 'polite')
  const header = node('div'); header.className = 'attachments-header'; header.append(heading, status)
  const controls = node('div'); controls.className = 'header-actions attachments-controls'; const toolbar = node('div'); toolbar.className = 'attachments-toolbar'; toolbar.append(header, controls); const content = node('div'); content.className = 'table-frame'; const confirmation = node('div'); confirmation.className = 'attachments-confirmation'; root.append(toolbar, content, confirmation); host.append(root)
  function button(text: string, callback: () => void, mutation = false): HTMLButtonElement { const result = node('button', text); result.type = 'button'; result.className = 'secondary-button'; result.disabled = busy || (mutation && (blocked || uncertain)); result.addEventListener('click', callback); return result }
  function report(error: unknown): void {
    if (error instanceof ApiError && error.code === 'selection_changed') { context.onSelectionChanged(); return }
    status.textContent = error instanceof ApiError ? `${apiGuidance(error)} 附件選擇已保留；不會自動重試。` : error instanceof Error ? error.message : '附件操作未完成。'
  }
  async function act(callback: (target: AttachmentEntry, signal: AbortSignal) => Promise<void>, mutation = false): Promise<void> {
    if (disposed || busy || !entry || (mutation && (blocked || uncertain))) return
    const token = generation; const target = { ...entry }; const controller = requestController
    busy = true; confirmation.replaceChildren(); context.onBusy(true); render()
    try { await callback(target, controller.signal) } catch (error) {
      if (disposed || token !== generation) return
      if (mutation && (!(error instanceof ApiError) || error.status === null || error.status === 409 || error.status >= 500)) uncertain = true
      report(error)
    } finally { if (!disposed && token === generation) { busy = false; context.onBusy(false); render() } }
  }
  async function load(signal: AbortSignal): Promise<void> {
    const token = generation; const target = entry; if (!target) return
    const next = parseAttachments(await context.request(`/api/journal/entries/${target.id}/attachments`, signal), target.id)
    if (disposed || token !== generation) return
    items = next; verified = new Set([...verified].filter(id => items.some(item => item.id === id)))
  }
  function render(): void {
    controls.replaceChildren(); content.replaceChildren(); confirmation.replaceChildren()
    if (!entry) { root.hidden = true; return }; root.hidden = false
    const picker = node('input'); picker.type = 'file'; picker.className = 'attachments-file-input'; picker.tabIndex = -1; picker.setAttribute('aria-label', '選擇分錄附件'); picker.multiple = false; picker.disabled = busy || blocked || uncertain || !context.upload
    picker.addEventListener('change', () => { selected = picker.files?.item(0) ?? null; confirmation.replaceChildren(); status.textContent = selected ? '已選擇檔案；按新增附件才會上傳。' : '尚未選擇檔案。'; context.onBusy(false); render() })
    const choose = button('選擇檔案', () => picker.click()); choose.className += ' attachments-choose'; choose.disabled = picker.disabled
    const filename = node('span', selected?.name ?? '尚未選擇檔案'); filename.className = 'attachments-selected-file'; filename.title = selected?.name ?? ''
    const upload = button('新增附件', () => {
      const file = selected; if (!file || !entry || !context.upload) { status.textContent = '請先選擇一般檔案。'; return }
      let path: string; try { path = uploadPath(entry, file) } catch (error) { report(error); return }
      confirmation.replaceChildren(node('p', entry.revision === null ? `確認新增「${file.name}」？已過帳附件只會追加，不能替換或移除。` : `確認將「${file.name}」新增至這份草稿？`), button('確認新增附件', () => void act(async (target, signal) => {
        const result = parseAttachment(await context.upload!(path, signal, file), target.id)
        if (result.displayName !== file.name || result.byteLength !== String(file.size) || items.some(item => item.id === result.id)) throw new Error('附件上傳回應與所選檔案不符；請重新載入核對。')
        await context.onEntryChanged(); await load(signal); selected = null; status.textContent = '附件已新增並重新讀取；可另按驗證確認內容完整性。'
      }, true), true), button('取消', () => confirmation.replaceChildren()))
    }, true)
    upload.disabled = picker.disabled || !selected
    const refresh = button('重新載入附件', () => void act(async (_target, signal) => { await context.onEntryChanged(); await load(signal); uncertain = false; status.textContent = '附件清單與分錄版本已重新載入；請核對有無前次新增結果再選擇檔案。' })); refresh.className += ' attachments-refresh'; refresh.setAttribute('aria-label', '重新載入附件'); refresh.title = '重新載入附件'
    controls.append(picker, choose, filename)
    if (selected) { const cancel = button('取消檔案選擇', () => { selected = null; status.textContent = '已取消尚未上傳的檔案選擇。'; render() }); cancel.className += ' attachments-cancel-selection'; controls.append(cancel) }
    const actions = node('div'); actions.className = 'attachments-right-actions'; actions.append(upload, refresh); controls.append(actions)
    const table = node('table'); const head = node('thead'); const headings = node('tr'); ['檔名', '大小（bytes）', '狀態', '操作'].forEach(label => headings.append(node('th', label))); head.append(headings); table.append(head); const body = node('tbody')
    items.forEach(item => { const row = node('tr'); const name = node('th', item.displayName); name.scope = 'row'; const size = node('td', BigInt(item.byteLength).toLocaleString('en-US')); size.className = 'amount-column'; const state = node('td', verified.has(item.id) ? '完整性已驗證' : '尚未驗證'); const actions = node('td')
      actions.append(button('驗證附件', () => void act(async (_target, signal) => { verifyAttachmentResult(await context.request(`/api/attachments/${item.id}/verify`, signal, 'POST'), item.id); verified.add(item.id); status.textContent = '附件內容完整性驗證通過。' })), button('下載附件', () => {
        if (!context.download) { status.textContent = '目前環境尚未提供安全串流下載。'; return }
        void act(async (_target, signal) => { await context.download!(`/api/attachments/${item.id}/content`, signal, item); status.textContent = '已交給 Safari 下載；請在 Safari 下載項目確認完成。檔案依 Safari 設定儲存至 Downloads。' })
      }))
      if (entry!.revision !== null) actions.append(button('移除附件', () => { confirmation.replaceChildren(node('p', `確認移除此草稿的「${item.displayName}」附件參照？`), button('確認移除附件', () => void act(async (target, signal) => { const command = detachCommand(target, item); const result = await context.request(command.path, signal, 'DELETE', command.payload); if (result !== null) throw new Error('附件移除回應不符；請重新載入核對。'); await context.onEntryChanged(); await load(signal); status.textContent = '草稿附件參照已移除。' }, true), true), button('取消', () => confirmation.replaceChildren())) }, true))
      row.append(name, size, state, actions); body.append(row)
    }); table.append(body); content.append(table, node('p', `${items.length} 份附件；每份上限 250 MiB，帳簿總量上限 20 GiB。已過帳附件不可替換或移除。${blocked ? ' 請先儲存或核對分錄後再變更附件。' : ''}`))
  }
  return {
    setEntry(next, nextBlocked) {
      if (disposed) return
      const changed = entry?.id !== next?.id || entry?.revision !== next?.revision
      entry = next ? { ...next } : null; blocked = nextBlocked
      if (changed && !busy) { generation++; requestController.abort(); requestController = new AbortController(); selected = null; uncertain = false; items = []; verified.clear(); render(); if (entry) void act(async (_target, signal) => { await load(signal); status.textContent = '附件清單已讀取。' }); return }
      render()
    },
    hasUnsavedChanges() { return selected !== null || busy || uncertain },
    dispose() { disposed = true; generation++; requestController.abort(); root.remove() },
  }
}
