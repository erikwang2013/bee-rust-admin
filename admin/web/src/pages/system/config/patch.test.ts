import { describe, expect, it } from 'vitest';
import { configPatch } from './patch';

/** 服务端一行的最小心跳：值 + 公开状态（0 私有 / 1 公开）。 */
const item = { value: '老值', is_public: 0 };

describe('configPatch', () => {
  it('只拨公开开关：只提交 is_public，不动 value', () => {
    expect(configPatch(item, '老值', true)).toEqual({ is_public: 1 });
  });

  it('只改值：只提交 value，不动公开状态', () => {
    expect(configPatch(item, '新值', false)).toEqual({ value: '新值' });
  });

  it('把已公开的关掉：is_public 归一成 0（而不是布尔 false）', () => {
    expect(configPatch({ value: 'x', is_public: 1 }, 'x', false)).toEqual({ is_public: 0 });
    expect(configPatch({ value: 'x', is_public: 1 }, 'y', false)).toEqual({ value: 'y', is_public: 0 });
  });

  it('值和开关都没变：返回 null（调用方据此提示「没有改动」、不发请求）', () => {
    expect(configPatch(item, '老值', false)).toBeNull();
    expect(configPatch({ value: 'x', is_public: 1 }, 'x', true)).toBeNull();
  });
});
