import { describe, expect, it } from 'vitest';
import { allIds, subtreeIds, toTreeData } from './tree';

interface Node {
  id: string;
  name: string;
  children?: Node[];
}

const tree: Node[] = [
  {
    id: '1', name: 'A',
    children: [
      { id: '2', name: 'B', children: [{ id: '3', name: 'C' }] },
      { id: '4', name: 'D' },
    ],
  },
  { id: '5', name: 'E' },
];

describe('subtreeIds', () => {
  it('含自身与全部子孙，不含祖先/兄弟', () => {
    expect([...subtreeIds(tree, '2')].sort()).toEqual(['2', '3']);
    expect([...subtreeIds(tree, '1')].sort()).toEqual(['1', '2', '3', '4']);
    expect([...subtreeIds(tree, '5')].sort()).toEqual(['5']);
  });

  it('不存在的 id 返回空集', () => {
    expect(subtreeIds(tree, 'nope').size).toBe(0);
  });
});

describe('allIds', () => {
  it('列出全部节点（深度优先）', () => {
    expect(allIds(tree)).toEqual(['1', '2', '3', '4', '5']);
  });
});

describe('toTreeData', () => {
  it('映射 value/title，blocked 里的节点 disabled', () => {
    expect(toTreeData(tree, new Set(['4']))).toEqual([
      {
        value: '1', title: 'A', disabled: false,
        children: [
          { value: '2', title: 'B', disabled: false, children: [{ value: '3', title: 'C', disabled: false, children: undefined }] },
          { value: '4', title: 'D', disabled: true, children: undefined },
        ],
      },
      { value: '5', title: 'E', disabled: false, children: undefined },
    ]);
  });
});
