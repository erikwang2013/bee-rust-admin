import { beforeEach, describe, expect, it, vi } from 'vitest';

// 拦掉 client（axios/antd 拦截器都不拉起）：只钉路径与请求体。
// 公告走 v2 方法组；路径仍以 /notices 开头，版本前缀由 httpV2 负责。
const { get, post, put, del } = vi.hoisted(() => ({
  get: vi.fn(), post: vi.fn(), put: vi.fn(), del: vi.fn(),
}));
vi.mock('./client', () => ({ httpV2: { get, post, put, del } }));

import { noticeApi } from './notice';

beforeEach(() => {
  get.mockReset();
  post.mockReset();
  put.mockReset();
  del.mockReset();
});

describe('noticeApi', () => {
  it('列表走摘要接口：查询条件原样透传', () => {
    const q = { page: 1, size: 10, status: 0 };
    noticeApi.list(q);

    expect(get).toHaveBeenCalledWith('/notices', q);
  });

  it('详情按 id 取全文（编辑回填 / 铃铛查看全文）', () => {
    noticeApi.detail('abc');

    expect(get).toHaveBeenCalledWith('/notices/abc');
  });

  it('发布 / 取消发布只传 status，不再回传整篇正文', () => {
    noticeApi.setStatus('abc', 1);

    expect(put).toHaveBeenCalledWith('/notices/abc/status', { status: 1 });
  });

  it('未读列表 + 标记已读路径不变', () => {
    noticeApi.unread();
    noticeApi.read('abc');

    expect(get).toHaveBeenCalledWith('/notices/unread');
    expect(post).toHaveBeenCalledWith('/notices/abc/read');
  });
});
