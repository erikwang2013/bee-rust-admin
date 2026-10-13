import type { TreeSelectProps } from 'antd';

/** TreeSelect 的数据项形状（toTreeData 的产物）。 */
export type TreeNode = NonNullable<TreeSelectProps['treeData']>[number];

/** 树节点的最小形状：id + name + 同类型 children（菜单树、部门树都满足）。 */
export interface TreeLike<T> {
  id: string;
  name: string;
  children?: T[];
}

/** 节点自身及其子孙的 id（父级选择里禁用，避免挂到自己下面成环）。 */
export function subtreeIds<T extends TreeLike<T>>(nodes: T[], id: string): Set<string> {
  const find = (list: T[]): T | undefined => {
    for (const n of list) {
      if (n.id === id) return n;
      const hit = n.children && find(n.children);
      if (hit) return hit;
    }
    return undefined;
  };
  const target = find(nodes);
  const out = new Set<string>();
  const walk = (n: T) => { out.add(n.id); n.children?.forEach(walk); };
  if (target) walk(target);
  return out;
}

/** 树里所有节点的 id（编辑时判断回显的 parent_id 是不是真节点）。 */
export function allIds<T extends TreeLike<T>>(nodes: T[]): string[] {
  return nodes.flatMap((n) => [n.id, ...allIds(n.children ?? [])]);
}

/** 树 → TreeSelect 数据；`blocked` 里的节点禁用。 */
export function toTreeData<T extends TreeLike<T>>(
  nodes: T[],
  blocked: Set<string> = new Set(),
): TreeNode[] {
  return nodes.map((n) => ({
    value: n.id,
    title: n.name,
    disabled: blocked.has(n.id),
    children: n.children?.length ? toTreeData(n.children, blocked) : undefined,
  }));
}
