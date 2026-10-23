import { flattenAccountTree, type AccountNode, type AccountingElement, type FlatAccountNode } from './account-tree.ts'

export interface AccountFilter {
  query: string
  element: AccountingElement | ''
  inactive: boolean
}

/** Search the complete ancestor path; retain context without changing sibling order. */
export function accountView(tree: AccountNode[], collapsed: ReadonlySet<string>, filter: AccountFilter): {
  rows: FlatAccountNode[]; matchingIds: Set<string>; paths: Map<string, AccountNode[]>
} {
  const paths = new Map<string, AccountNode[]>()
  const matchingIds = new Set<string>()
  const included = new Set<string>()
  const query = filter.query.trim().toLocaleLowerCase('zh-TW')
  const pending = tree.map(node => ({ node, ancestors: [] as AccountNode[] })).reverse()
  while (pending.length) {
    const { node, ancestors } = pending.pop()!
    const path = [...ancestors, node]
    paths.set(node.account.id, path)
    if (node.account.isActive !== filter.inactive &&
        (!filter.element || node.account.element === filter.element) &&
        path.map(item => item.account.name).join(' / ').toLocaleLowerCase('zh-TW').includes(query)) {
      matchingIds.add(node.account.id)
      path.forEach(item => included.add(item.account.id))
    }
    for (let index = node.children.length - 1; index >= 0; index -= 1) {
      pending.push({ node: node.children[index]!, ancestors: path })
    }
  }
  // Search and inactive results reveal matching descendants even under a collapsed ancestor.
  const reveal = query !== '' || filter.element !== '' || filter.inactive
  return {
    rows: flattenAccountTree(tree, reveal ? new Set() : collapsed).filter(row => included.has(row.account.id)),
    matchingIds, paths,
  }
}
