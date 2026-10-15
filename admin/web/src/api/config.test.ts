import { beforeEach, describe, expect, it, vi } from 'vitest';

// 拦掉 client（axios/antd 拦截器都不拉起）：只钉 fetchPublicConfig 自己的共享/释放语义
const { get } = vi.hoisted(() => ({ get: vi.fn() }));
vi.mock('./client', () => ({ http: { get } }));

import { fetchPublicConfig } from './config';

describe('fetchPublicConfig', () => {
  beforeEach(() => {
    get.mockReset();
  });

  it('并发调用共享同一个 in-flight 请求（登录页副标题 + 页脚同时挂载）', async () => {
    let resolve!: (v: unknown) => void;
    get.mockReturnValue(new Promise((r) => { resolve = r; }));

    const a = fetchPublicConfig();
    const b = fetchPublicConfig();
    expect(get).toHaveBeenCalledTimes(1);

    resolve({ 'site.repo_url': 'https://example.com' });
    await expect(a).resolves.toEqual({ 'site.repo_url': 'https://example.com' });
    await expect(b).resolves.toEqual({ 'site.repo_url': 'https://example.com' });
  });

  it('resolve 后释放：下一次调用重新取数（改配置 → 刷新即变，不许有缓存）', async () => {
    get.mockResolvedValueOnce({ v: 1 });
    await expect(fetchPublicConfig()).resolves.toEqual({ v: 1 });

    get.mockResolvedValueOnce({ v: 2 });
    await expect(fetchPublicConfig()).resolves.toEqual({ v: 2 });
    expect(get).toHaveBeenCalledTimes(2);
  });

  it('失败也释放：不会卡在失败态（后端恢复后下一次就能取到）', async () => {
    get.mockRejectedValueOnce(new Error('网络错误'));
    await expect(fetchPublicConfig()).rejects.toThrow('网络错误');

    get.mockResolvedValueOnce({ ok: true });
    await expect(fetchPublicConfig()).resolves.toEqual({ ok: true });
  });
});
