import { http } from './client';
import type { DashboardStats } from './types';

export const dashboardApi = {
  /** 登录即可（不挂权限码）；近 7 天序列已补 0，旧→新 */
  stats: () => http.get<DashboardStats>('/dashboard/stats'),
};
