import type { NoticeUnread } from '../api/types';

/**
 * 标记已读成功后的本地更新：角标减一 + 条目移出列表。
 *
 * **失败时调用方不要调它** —— 条目留在列表里，用户能重试（乐观移除会让失败静默丢条目）。
 * 减的是接口给的 total 而不是 list.length：list 只给最近 50 条，超过就少报了。
 */
export function applyRead(state: NoticeUnread, id: string): NoticeUnread {
  if (!state.list.some((n) => n.id === id)) return state; // 重复点击 / 已被别处刷新：原样不动
  return { total: Math.max(0, state.total - 1), list: state.list.filter((n) => n.id !== id) };
}
