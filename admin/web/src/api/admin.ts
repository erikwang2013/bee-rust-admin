import { http } from './client';
import type { Admin, Page } from './types';

/** 筛选用的 dept_id：空串 = 不筛（后端当缺省） */
export interface AdminQuery { page?: number; size?: number; username?: string; status?: number; dept_id?: string }
export interface AdminForm {
  username?: string; password?: string; nickname: string; email: string; phone: string;
  sex: number; dept_id: string; status: number; remark: string; role_ids: string[];
}

export const adminApi = {
  list: (q: AdminQuery) => http.get<Page<Admin>>('/admins', q),
  get: (id: string) => http.get<Admin>(`/admins/${id}`),
  create: (data: AdminForm) => http.post<Admin>('/admins', data),
  update: (id: string, data: AdminForm) => http.put<Admin>(`/admins/${id}`, data),
  remove: (id: string) => http.del<null>(`/admins/${id}`),
  setStatus: (id: string, status: number) => http.put<null>(`/admins/${id}/status`, { status }),
  resetPassword: (id: string, password: string) => http.put<null>(`/admins/${id}/password`, { password }),
  setRoles: (id: string, role_ids: string[]) => http.put<null>(`/admins/${id}/roles`, { role_ids }),
};
