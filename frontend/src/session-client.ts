export interface SessionStatus { bookId: string | null; selectionRequired: boolean; selectionId: string | null }

function object(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('帳簿回應格式不符')
  return value as Record<string, unknown>
}
export function validBookId(value: unknown): value is string {
  return typeof value === 'string' && value.length > 0 && value.length <= 255 &&
    !value.startsWith('.') && !/[\\/\u0000-\u001f\u007f-\u009f]/.test(value)
}
export function parseSession(value: unknown): SessionStatus {
  const source = object(value)
  if (typeof source.selection_required !== 'boolean' ||
      !(source.book_id === null || validBookId(source.book_id)) ||
      source.selection_required !== (source.book_id === null) ||
      !(source.selection_id === null || (typeof source.selection_id === 'string' && /^[a-f0-9]{32}$/.test(source.selection_id))) ||
      (source.selection_id === null) !== (source.book_id === null)) throw new Error('帳簿選擇狀態不一致')
  return { bookId: source.book_id, selectionRequired: source.selection_required, selectionId: source.selection_id }
}
export function parseBooks(value: unknown): string[] {
  const source = object(value)
  if (!Array.isArray(source.books) || source.books.length > 5_000) throw new Error('帳簿清單格式不符')
  const seen = new Set<string>()
  return source.books.map(item => {
    const book = object(item)
    if (!validBookId(book.id) || book.status !== 'selection_required' || seen.has(book.id)) throw new Error('帳簿識別不符')
    seen.add(book.id)
    return book.id
  })
}

export class ApiError extends Error {
  code: string
  status: number | null
  constructor(code: string, status: number | null = null) { super('無法完成要求'); this.code = code; this.status = status }
}

const guidance: Record<string, string> = {
  book_busy: '帳簿仍有處理中的作業，請稍後再試。',
  book_not_available: '帳簿已無法使用，請重新整理清單。',
  book_collection_unavailable: '目前無法讀取帳簿清單。',
  book_open_failed: '無法開啟帳簿，請檢查帳簿與金鑰狀態。',
  selection_required: '請先選擇帳簿。',
  selection_changed: '帳簿已在其他分頁切換；已重新確認目前帳簿。',
  session_closed: '服務已關閉，請重新啟動服務。',
  keychain_interaction_required: 'Keychain 需要你完成系統授權或解鎖，完成後再試。',
  keychain_user_cancelled: '已取消 Keychain 授權；帳簿未開啟。',
  keychain_unavailable: 'Keychain 目前無法使用，請檢查系統狀態後再試。',
  missing_original_key: '找不到此帳簿原有的金鑰；請保留帳簿並檢查原金鑰。',
  write_in_progress: '變更仍在處理中，請等候結果後再操作。',
  account_revision_conflict: '科目已由其他操作更新。輸入已保留，請重新載入後檢視。',
  draft_revision_conflict: '草稿版本已改變。輸入已保留，請重新載入後檢視。',
  duplicate_account_name: '同一層已有相同科目名稱，請修改名稱。',
  account_rule_violation: '操作不符合科目階層、啟用或歷史資料規則。請檢查後再試。',
  journal_rule_violation: '分錄不符合過帳規則。請核對借貸平衡、科目狀態及日期。',
  invalid_journal_input: '分錄欄位格式不符，請檢查日期、科目及整數金額。',
  journal_state_conflict: '分錄狀態已改變，請重新載入；不會修改已過帳分錄。',
  correction_explanation_required: '更正分錄需要填寫說明。',
  native_picker_busy: '另一個原生選擇視窗仍開啟，請先完成或取消。',
  native_selection_failed: '原生選擇未完成；請檢查選取的檔案種類與私有目的地。',
  invalid_native_book_destination: '請在目前帳簿集合內輸入新的帳簿名稱；既有帳簿不會被覆寫。',
  invalid_native_selection: '原生選擇要求格式不符，請重新載入介面。',
  native_host_closed: '原生服務已關閉，請重新啟動本機版本。',
  backup_identity_missing: '此帳簿尚未設定本機備份；請明確完成備份身分設定，再建立備份。',
  payment_memo_invalid: '付款計畫欄位無效，請核對日期、文字與正整數金額。',
  payment_memo_conflict: '付款計畫或付款草稿狀態已變更；請重新查詢核對，不要重送。',
  payment_memo_not_found: '找不到這筆付款計畫；請重新查詢。',
  payment_memo_overflow: '付款計畫合計超過整數範圍，請縮小查詢期間並核對金額。',
  open_item_invalid: '款項或分配欄位無效，請核對來源與正整數金額。',
  open_item_not_found: '找不到所選已過帳分錄行或款項；請重新核對。',
  open_item_overflow: '款項合計超過整數範圍，請核對來源金額。',
  open_item_reversed: '來源已沖銷或本身是沖銷分錄，不能再建立款項或分配。',
  open_item_wrong_account: '款項與收付款須為同科目，且借貸方向相反。',
  open_item_exists: '這個來源行已登記款項，請查詢既有明細，不要重複登記。',
  open_item_split_mismatch: '款項金額合計必須等於所選來源行。',
  open_item_overallocated: '分配超過款項未結額或收付款行未分配額，請重新核對。',
  open_item_date_order: '收付款或調整日期不能早於原款項日期。',
  open_item_allocation_inactive: '此分配已解除或不存在，請重新查詢歷史。',
  open_item_allocations_active: '此分錄仍有有效收付分配；請先核對並明確解除分配，再沖銷。',
  backup_identity_exists: '此帳簿已有備份身分，未被替換；請核對目前狀態後再建立備份。',
  backup_identity_invalid: '備份身分無法驗證；請保留原檔與帳簿，不會覆寫或自動重建。',
  backup_identity_initialization_failed: '備份身分初始化結果未確認；請先重新查詢狀態，不要重複提交。',
  backup_busy: '備份維護仍在進行，請稍後再試。',
  storage_busy: '另一個儲存作業仍在進行，請先查詢該作業結果。',
  storage_capacity: '儲存作業已達容量限制；請保留來源並檢查可用空間。',
  storage_operation_failed: '儲存作業失敗；請保留來源及中斷工作，核對後再復原。',
  storage_operation_not_found: '找不到這次程序的工作紀錄，服務可能已重啟；請保留來源，明確檢查中斷復原，不要重送原操作。',
  posted_attachment_immutable: '已過帳附件不可替換或移除；可另行新增附件。',
  attachment_integrity_failed: '附件完整性驗證未通過，請保留來源與備份。',
}
export function apiGuidance(error: unknown): string {
  return error instanceof ApiError ? guidance[error.code] ?? '服務無法完成要求，請重新整理後再試。' : '無法安全讀取服務回應，請確認服務可用後再試。'
}

export async function requestJson(path: string, signal: AbortSignal, method = 'GET', payload?: unknown, selection?: string): Promise<unknown> {
  // File is handed directly to the browser transport; never materialize bytes.
  const binary = typeof File !== 'undefined' && payload instanceof File
  const response = await fetch(path, {
    method, credentials: 'same-origin', cache: 'no-store', redirect: 'error', signal,
    headers: { Accept: 'application/json', ...(payload === undefined ? {} : { 'Content-Type': binary ? 'application/octet-stream' : 'application/json' }), ...(selection === undefined ? {} : { 'X-kAccounting-Selection': selection }) },
    ...(payload === undefined ? {} : { body: binary ? payload as File : JSON.stringify(payload) }),
  })
  if (response.status === 204 && response.ok) return null
  if (!(response.headers.get('content-type') ?? '').toLowerCase().includes('application/json')) throw new Error('回應不是 JSON')
  const value: unknown = await response.json()
  if (!response.ok) {
    const source = object(value)
    const error = source.error && typeof source.error === 'object' ? object(source.error) : source
    throw new ApiError(typeof error.code === 'string' ? error.code : 'request_failed', response.status)
  }
  return value
}

export function nativeDownloadUrl(path: string, selection: string): string {
  if (!/^\/api\/(?:attachments\/[a-f0-9]{32}\/content|storage\/operations\/[a-f0-9]{32}\/download)$/.test(path) || !/^[a-f0-9]{32}$/.test(selection)) throw new Error('下載識別不符')
  return `${path}?download_selection=${selection}`
}

/** Hand off to Safari's bounded native download, never claim the file is saved. */
export function startNativeDownload(path: string, selection: string): void {
  const anchor = document.createElement('a')
  anchor.href = nativeDownloadUrl(path, selection)
  anchor.download = ''; anchor.rel = 'noopener'; anchor.hidden = true
  document.body.append(anchor)
  try { anchor.click() } finally { anchor.remove() }
}
