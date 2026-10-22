export type AccountingElement =
  | 'asset'
  | 'liability'
  | 'equity'
  | 'income'
  | 'expense'

export type AccountKind = 'category' | 'posting'

export interface AccountRecord {
  id: string
  parentId: string | null
  name: string
  element: AccountingElement
  kind: AccountKind
  isActive: boolean
  revision: string
  createdAt: string
  updatedAt: string
}

export interface AccountNode {
  account: AccountRecord
  children: AccountNode[]
}

export interface FlatAccountNode {
  account: AccountRecord
  depth: number
  hasChildren: boolean
}

export interface AccountNameSegment {
  text: string
  script: 'han' | 'system'
}

export class AccountTreeResponseError extends Error {
  override name = 'AccountTreeResponseError'
}

const elements = new Set<AccountingElement>([
  'asset',
  'liability',
  'equity',
  'income',
  'expense',
])

const kinds = new Set<AccountKind>(['category', 'posting'])
const maximumI64 = '9223372036854775807'
const maximumTreeDepth = 64
const maximumTreeNodes = 5_000
const hanCharacter = /\p{Script=Han}/u
const controlCharacter = /[\u0000-\u001f\u007f-\u009f]/
const edgeWhitespaceCharacter = /^\p{White_Space}|\p{White_Space}$/u

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function readString(
  record: Record<string, unknown>,
  key: string,
  path: string,
): string {
  const value = record[key]
  if (typeof value !== 'string' || value.length === 0) {
    throw new AccountTreeResponseError(`${path}.${key} 必須是非空字串`)
  }
  return value
}

export function isPositiveI64Decimal(value: string): boolean {
  if (!/^[1-9]\d*$/.test(value)) return false
  if (value.length !== maximumI64.length) return value.length < maximumI64.length
  return value <= maximumI64
}

function readExactIntegerString(
  record: Record<string, unknown>,
  key: string,
  path: string,
): string {
  const value = readString(record, key, path)
  if (!isPositiveI64Decimal(value)) {
    throw new AccountTreeResponseError(`${path}.${key} 必須是正整數十進位字串`)
  }
  return value
}

function readAccountName(record: Record<string, unknown>, path: string): string {
  const name = readString(record, 'name', path)
  const characterCount = Array.from(name).length
  if (
    edgeWhitespaceCharacter.test(name) ||
    characterCount > 120 ||
    controlCharacter.test(name)
  ) {
    throw new AccountTreeResponseError(`${path}.name 不符合科目名稱格式`)
  }
  return name
}

function parseAccount(
  value: Record<string, unknown>,
  expectedParentId: string | null,
  expectedElement: AccountingElement | null,
  path: string,
  hasChildren: boolean,
  seenIds: Set<string>,
): AccountRecord {
  if (!isRecord(value.account)) {
    throw new AccountTreeResponseError(`${path}.account 必須是物件`)
  }

  const source = value.account
  const id = readExactIntegerString(source, 'id', `${path}.account`)
  if (seenIds.has(id)) {
    throw new AccountTreeResponseError(`${path}.account.id 重複`)
  }
  seenIds.add(id)

  const parentValue = source.parent_id
  if (parentValue !== null && typeof parentValue !== 'string') {
    throw new AccountTreeResponseError(`${path}.account.parent_id 必須是字串或 null`)
  }
  if (parentValue !== null && !isPositiveI64Decimal(parentValue)) {
    throw new AccountTreeResponseError(
      `${path}.account.parent_id 必須是正整數十進位字串`,
    )
  }
  if (parentValue !== expectedParentId) {
    throw new AccountTreeResponseError(`${path}.account.parent_id 與科目樹階層不一致`)
  }

  const element = readString(source, 'element', `${path}.account`)
  if (!elements.has(element as AccountingElement)) {
    throw new AccountTreeResponseError(`${path}.account.element 不受支援`)
  }
  if (expectedElement !== null && element !== expectedElement) {
    throw new AccountTreeResponseError(
      `${path}.account.element 與父科目的會計要素不一致`,
    )
  }

  const kind = readString(source, 'kind', `${path}.account`)
  if (!kinds.has(kind as AccountKind)) {
    throw new AccountTreeResponseError(`${path}.account.kind 不受支援`)
  }
  if (kind === 'posting' && hasChildren) {
    throw new AccountTreeResponseError(`${path} 的過帳科目不可包含子科目`)
  }
  const isActive = source.is_active
  if (typeof isActive !== 'boolean') {
    throw new AccountTreeResponseError(`${path}.account.is_active 必須是布林值`)
  }

  return {
    id,
    parentId: parentValue,
    name: readAccountName(source, `${path}.account`),
    element: element as AccountingElement,
    kind: kind as AccountKind,
    isActive,
    revision: readExactIntegerString(source, 'revision', `${path}.account`),
    createdAt: readString(source, 'created_at', `${path}.account`),
    updatedAt: readString(source, 'updated_at', `${path}.account`),
  }
}

export function parseAccountTree(value: unknown): AccountNode[] {
  if (!Array.isArray(value)) throw new AccountTreeResponseError('科目樹回應必須是陣列')
  if (value.length > maximumTreeNodes) {
    throw new AccountTreeResponseError(`科目樹超過顯示數量上限 ${maximumTreeNodes}`)
  }
  type EnterTask = {
    stage: 'enter'
    value: unknown
    expectedParentId: string | null
    expectedElement: AccountingElement | null
    depth: number
    path: string
    destination: AccountNode[]
  }
  type LeaveTask = { stage: 'leave'; value: object }

  const roots: AccountNode[] = []
  const pending: Array<EnterTask | LeaveTask> = []
  const seenIds = new Set<string>()
  const ancestors = new WeakSet<object>()
  // Count nodes when discovered so oversized (including sparse) child arrays
  // fail before they can fill the traversal stack.
  let discoveredNodeCount = value.length

  for (let index = value.length - 1; index >= 0; index -= 1) {
    pending.push({
      stage: 'enter',
      value: value[index],
      expectedParentId: null,
      expectedElement: null,
      depth: 0,
      path: `科目樹[${index}]`,
      destination: roots,
    })
  }

  while (pending.length > 0) {
    const task = pending.pop()!
    if (task.stage === 'leave') {
      ancestors.delete(task.value)
      continue
    }

    if (task.depth > maximumTreeDepth) {
      throw new AccountTreeResponseError(`科目樹超過顯示深度上限 ${maximumTreeDepth}`)
    }
    if (!isRecord(task.value)) {
      throw new AccountTreeResponseError(`${task.path} 必須是物件`)
    }
    if (ancestors.has(task.value)) {
      throw new AccountTreeResponseError(`${task.path} 含有循環參照`)
    }
    ancestors.add(task.value)

    if (!Array.isArray(task.value.children)) {
      throw new AccountTreeResponseError(`${task.path}.children 必須是陣列`)
    }
    const sourceChildren = task.value.children
    if (discoveredNodeCount + sourceChildren.length > maximumTreeNodes) {
      throw new AccountTreeResponseError(`科目樹超過顯示數量上限 ${maximumTreeNodes}`)
    }
    discoveredNodeCount += sourceChildren.length

    const parsedAccount = parseAccount(
      task.value,
      task.expectedParentId,
      task.expectedElement,
      task.path,
      sourceChildren.length > 0,
      seenIds,
    )
    const parsedNode: AccountNode = { account: parsedAccount, children: [] }
    task.destination.push(parsedNode)

    // A leave task gives iterative DFS the same active-ancestor boundary as
    // unwinding a recursive call, without consuming the JavaScript call stack.
    pending.push({ stage: 'leave', value: task.value })
    for (let index = sourceChildren.length - 1; index >= 0; index -= 1) {
      pending.push({
        stage: 'enter',
        value: sourceChildren[index],
        expectedParentId: parsedAccount.id,
        expectedElement: parsedAccount.element,
        depth: task.depth + 1,
        path: `${task.path}.children[${index}]`,
        destination: parsedNode.children,
      })
    }
  }

  return roots
}

export function flattenAccountTree(
  nodes: AccountNode[],
  collapsedIds: ReadonlySet<string>,
): FlatAccountNode[] {
  const rows: FlatAccountNode[] = []
  const pending: Array<{ node: AccountNode; depth: number }> = []
  for (let index = nodes.length - 1; index >= 0; index -= 1) {
    pending.push({ node: nodes[index]!, depth: 0 })
  }

  while (pending.length > 0) {
    const { node, depth } = pending.pop()!
    rows.push({
      account: node.account,
      depth,
      hasChildren: node.children.length > 0,
    })
    if (!collapsedIds.has(node.account.id)) {
      for (let index = node.children.length - 1; index >= 0; index -= 1) {
        pending.push({ node: node.children[index]!, depth: depth + 1 })
      }
    }
  }
  return rows
}

export function segmentAccountName(name: string): AccountNameSegment[] {
  const segments: AccountNameSegment[] = []

  for (const character of name) {
    const script = hanCharacter.test(character) ? 'han' : 'system'
    const previous = segments[segments.length - 1]
    if (previous?.script === script) {
      previous.text += character
    } else {
      segments.push({ text: character, script })
    }
  }

  return segments
}
