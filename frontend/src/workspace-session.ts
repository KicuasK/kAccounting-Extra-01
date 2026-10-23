import { ApiError } from './session-client.ts'

export interface SelectionSnapshot { bookId: string; selectionId: string }
export type WorkspaceTransport = (path: string, signal: AbortSignal, method?: string, payload?: unknown, selection?: string) => Promise<unknown>
export function sameSnapshot(left: SelectionSnapshot | null, right: SelectionSnapshot | null): boolean {
  return left !== null && right !== null && left.bookId === right.bookId && left.selectionId === right.selectionId
}

/** Each mounted editor/report owns an immutable selection and never rebinds a write. */
export function pinnedWorkspaceRequest(
  selection: SelectionSnapshot,
  current: () => SelectionSnapshot | null,
  transport: WorkspaceTransport,
  writeState: (delta: 1 | -1) => void,
): (path: string, signal: AbortSignal, method?: string, payload?: unknown) => Promise<unknown> {
  let writing = false
  return async (path, signal, method = 'GET', payload) => {
    if (!/^\/api\/(?!session(?:\/|$))[^\\#\r\n]*$/.test(path)) throw new Error('不允許的工作區路徑')
    if (!sameSnapshot(selection, current())) throw new ApiError('selection_changed')
    if (signal.aborted) throw new DOMException('已取消要求', 'AbortError')
    const mutation = !['GET', 'HEAD'].includes(method)
    if (mutation && writing) throw new ApiError('write_in_progress')
    if (mutation) { writing = true; writeState(1) }
    try {
      const result = await transport(path, signal, method, payload, selection.selectionId)
      if (!sameSnapshot(selection, current())) throw new ApiError('selection_changed')
      return result
    } finally {
      if (mutation) { writing = false; writeState(-1) }
    }
  }
}
