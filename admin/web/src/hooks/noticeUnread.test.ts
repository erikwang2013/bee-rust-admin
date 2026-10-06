import { describe, expect, it } from 'vitest';
import { applyRead } from './noticeUnread';
import type { Notice, NoticeUnread } from '../api/types';

const notice = (id: number): Notice => ({
  id, title: `t${id}`, content: '', status: 1, created_by: 1,
  published_at: '2026-10-06 10:00:00', created_at: '', updated_at: '',
});

describe('applyRead', () => {
  it('标记已读成功后：角标减一 + 条目移出列表', () => {
    const s: NoticeUnread = { total: 5, list: [notice(3), notice(2), notice(1)] };

    expect(applyRead(s, 2)).toEqual({ total: 4, list: [notice(3), notice(1)] });
  });

  it('角标用接口的 total 而不是 list.length：超过 50 条时也得减一', () => {
    const list = Array.from({ length: 50 }, (_, i) => notice(100 - i));
    const next = applyRead({ total: 120, list }, 100);

    expect(next.total).toBe(119);
    expect(next.list).toHaveLength(49);
  });

  it('条目不在列表里（重复点击 / 被别处刷新过）时原样返回', () => {
    const s: NoticeUnread = { total: 5, list: [notice(1)] };

    expect(applyRead(s, 9)).toBe(s);
  });

  it('total 与列表不一致时也不会减成负数', () => {
    const next = applyRead({ total: 0, list: [notice(1)] }, 1);

    expect(next.total).toBe(0);
    expect(next.list).toHaveLength(0);
  });

  it('不改动传入的状态对象', () => {
    const s: NoticeUnread = { total: 2, list: [notice(1), notice(2)] };

    applyRead(s, 1);

    expect(s.total).toBe(2);
    expect(s.list).toHaveLength(2);
  });
});
