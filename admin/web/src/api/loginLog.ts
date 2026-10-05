import { http } from './client';
import type { LoginLog, Page } from './types';

export interface LoginLogQuery {
  page?: number; size?: number; username?: string; status?: number; start?: string; end?: string;
}

export const loginLogApi = {
  list: (q: LoginLogQuery) => http.get<Page<LoginLog>>('/login-logs', q),
  clear: (q: LoginLogQuery) => http.del<null>('/login-logs', q),
};
