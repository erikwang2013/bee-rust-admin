import { beforeEach, describe, expect, it, vi } from 'vitest';
import { dictCache, loadDict } from './useDict';

// useDict 组件本身要 jsdom/RTL 才能测；这里只测它能被抽出来的部分：缓存键与失败不缓存。
const { items } = vi.hoisted(() => ({ items: vi.fn() }));
vi.mock('../api/dict', () => ({ dictApi: { items } }));

const SEX = [{ label: '男', value: '1' }, { label: '女', value: '0' }];

beforeEach(() => {
  items.mockReset();
  dictCache.clear();
});

describe('loadDict', () => {
  it('同一个 code 只请求一次，后到的拿缓存', async () => {
    items.mockResolvedValue(SEX);

    expect(await loadDict('user_sex')).toEqual(SEX);
    expect(await loadDict('user_sex')).toEqual(SEX);

    expect(items).toHaveBeenCalledTimes(1);
    expect(items).toHaveBeenCalledWith('user_sex');
  });

  it('并发调用共用同一个请求', async () => {
    items.mockResolvedValue(SEX);

    const [a, b] = await Promise.all([loadDict('user_sex'), loadDict('user_sex')]);

    expect(items).toHaveBeenCalledTimes(1);
    expect(a).toBe(b);
  });

  it('按 code 分键，不同字典互不影响', async () => {
    items.mockImplementation((code: string) => Promise.resolve([{ label: code, value: code }]));

    expect(await loadDict('a')).toEqual([{ label: 'a', value: 'a' }]);
    expect(await loadDict('b')).toEqual([{ label: 'b', value: 'b' }]);

    expect(items).toHaveBeenCalledTimes(2);
    expect(dictCache.size).toBe(2);
  });

  it('失败的请求不留缓存，下次重新拉', async () => {
    items.mockRejectedValueOnce(new Error('boom')).mockResolvedValue(SEX);

    await expect(loadDict('user_sex')).rejects.toThrow('boom');
    expect(dictCache.has('user_sex')).toBe(false);

    expect(await loadDict('user_sex')).toEqual(SEX);
    expect(items).toHaveBeenCalledTimes(2);
  });
});
