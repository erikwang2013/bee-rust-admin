import { http } from './client';
import type { Notice, NoticeUnread, Page } from './types';

export interface NoticeQuery { page?: number; size?: number; title?: string; status?: number }
export interface NoticeForm { title: string; content: string; status: number }

export const noticeApi = {
  list: (q: NoticeQuery) => http.get<Page<Notice>>('/notices', q),
  /** 后端只回新建出来的 id（正文不回传，列表页刷新时再取） */
  create: (data: NoticeForm) => http.post<{ id: string }>('/notices', data),
  update: (id: string, data: NoticeForm) => http.put<null>(`/notices/${id}`, data),
  remove: (id: string) => http.del<null>(`/notices/${id}`),

  /** 未读列表：只认登录不挂权限码（每个登录用户都要有的能力） */
  unread: () => http.get<NoticeUnread>('/notices/unread'),
  /** 标记已读，幂等；目标是草稿时后端 404 */
  read: (id: string) => http.post<null>(`/notices/${id}/read`),
};
