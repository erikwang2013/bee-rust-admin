import { http } from './client';
import type { NotifyConfig } from './types';

/**
 * 存配置的提交体：两个密钥字段都是**缺省 = 不改**（后端口径）。
 * 所以输入框留空时调用方要把字段整个删掉，别提交空串 —— 空串是「清空」。
 */
export type NotifyForm = Omit<NotifyConfig, 'smtp_password_set' | 'webhook_secret_set'> & {
  smtp_password?: string;
  webhook_secret?: string;
};

export const notifyApi = {
  get: () => http.get<NotifyConfig>('/notify/config'),
  save: (data: NotifyForm) => http.put<null>('/notify/config', data),
  /** 发测试邮件给当前登录者自己的邮箱；失败时后端回 400 + 原因（拦截器会提示） */
  testEmail: () => http.post<{ to: string }>('/notify/test-email', {}),
};
