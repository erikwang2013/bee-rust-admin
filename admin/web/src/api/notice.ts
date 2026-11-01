import { httpV2 as http } from './client';
import type { Notice, NoticeSummary, NoticeUnread, Page } from './types';

export interface NoticeQuery { page?: number; size?: number; title?: string; status?: number }
export interface NoticeForm { title: string; content: string; status: number }

export const noticeApi = {
  /** 列表只回摘要（不含正文）：编辑时再按 id 取详情 */
  list: (q: NoticeQuery) => http.get<Page<NoticeSummary>>('/notices', q),
  detail: (id: string) => http.get<Notice>(`/notices/${id}`),
  /** 后端只回新建出来的 id（正文不回传，列表页刷新时再取） */
  create: (data: NoticeForm) => http.post<{ id: string }>('/notices', data),
  update: (id: string, data: NoticeForm) => http.put<null>(`/notices/${id}`, data),
  /** 发布 / 取消发布只传状态，不再回传整篇正文 */
  setStatus: (id: string, status: number) => http.put<null>(`/notices/${id}/status`, { status }),
  remove: (id: string) => http.del<null>(`/notices/${id}`),

  /** 未读列表：只认登录不挂权限码（每个登录用户都要有的能力）；行是摘要 */
  unread: () => http.get<NoticeUnread>('/notices/unread'),
  /** 标记已读，幂等；目标是草稿时后端 404 */
  read: (id: string) => http.post<null>(`/notices/${id}/read`),
};
