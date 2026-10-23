import { isPositiveI64Decimal } from './account-tree.ts'
export const MAX_ATTACHMENT_BYTES = 262144000n
export interface AttachmentMetadata { id: string; entryId: string; displayName: string; byteLength: string; sha256: string; createdAt: string }
export interface AttachmentEntry { id: string; revision: string | null }
function object(value: unknown): Record<string, unknown> { if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('附件回應格式不符。'); return value as Record<string, unknown> }
export function validAttachmentName(name: string): boolean { return name.length > 0 && Array.from(name).length <= 255 && name !== '.' && name !== '..' && !/[\\/\u0000-\u001f\u007f-\u009f]/.test(name) }
export function parseAttachment(value: unknown, entryId: string): AttachmentMetadata {
  const source = object(value)
  if (typeof source.id !== 'string' || !/^[a-f0-9]{32}$/.test(source.id) || source.entry_id !== entryId || !isPositiveI64Decimal(entryId) || typeof source.display_name !== 'string' || !validAttachmentName(source.display_name) || typeof source.byte_length !== 'string' || !/^(0|[1-9]\d*)$/.test(source.byte_length) || BigInt(source.byte_length) > MAX_ATTACHMENT_BYTES || typeof source.sha256 !== 'string' || !/^[a-f0-9]{64}$/.test(source.sha256) || typeof source.created_at !== 'string' || !source.created_at) throw new Error('附件識別、名稱或完整性資料不符。')
  return { id: source.id, entryId, displayName: source.display_name, byteLength: source.byte_length, sha256: source.sha256, createdAt: source.created_at }
}
export function parseAttachments(value: unknown, entryId: string): AttachmentMetadata[] {
  if (!Array.isArray(value) || value.length > 10000) throw new Error('附件清單格式或筆數不符。')
  const seen = new Set<string>(); return value.map(raw => { const item = parseAttachment(raw, entryId); if (seen.has(item.id)) throw new Error('附件識別重複。'); seen.add(item.id); return item })
}
export function uploadPath(entry: AttachmentEntry, file: { name: string; size: number; webkitRelativePath?: string }): string {
  if (!isPositiveI64Decimal(entry.id) || (entry.revision !== null && !isPositiveI64Decimal(entry.revision))) throw new Error('分錄識別或版本不符。')
  if (!validAttachmentName(file.name) || file.webkitRelativePath || !Number.isSafeInteger(file.size) || file.size < 0 || BigInt(file.size) > MAX_ATTACHMENT_BYTES) throw new Error('請選擇一般檔案，名稱不可含路徑，每份不超過 250 MiB。')
  if (/\.(app|pkg|mpkg|bundle|framework|kext|plugin|workflow|action|exe|dll|com|bat|cmd|command|sh|bash|zsh|ps1|scpt|applescript)$/i.test(file.name)) throw new Error('不可上傳執行檔、指令或 macOS 套件。')
  return `/api/journal/entries/${entry.id}/attachments?display_name=${encodeURIComponent(file.name)}&source_kind=regular_file${entry.revision === null ? '' : `&expected_revision=${entry.revision}`}`
}
export function detachCommand(entry: AttachmentEntry, attachment: AttachmentMetadata): { path: string; payload: { expected_revision: string } } {
  if (entry.revision === null || !isPositiveI64Decimal(entry.revision) || entry.id !== attachment.entryId) throw new Error('已過帳附件不可移除或替換。')
  return { path: `/api/attachments/${attachment.id}`, payload: { expected_revision: entry.revision } }
}
export function verifyAttachmentResult(value: unknown, id: string): void { const source = object(value); if (source.id !== id || source.verified !== true) throw new Error('附件驗證回應不符。') }
