import { http } from './client';
import type { Admin, Page } from './types';

export interface AdminQuery { page?: number; size?: number; username?: string; status?: number; dept_id?: number }
export interface AdminForm {
  username?: string; password?: string; nickname: string; email: string; phone: string;
  sex: number; dept_id: number; status: number; remark: string; role_ids: number[];
}

export const adminApi = {
  list: (q: AdminQuery) => http.get<Page<Admin>>('/admins', q),
  get: (id: number) => http.get<Admin>(`/admins/${id}`),
  create: (data: AdminForm) => http.post<Admin>('/admins', data),
  update: (id: number, data: AdminForm) => http.put<Admin>(`/admins/${id}`, data),
  remove: (id: number) => http.del<null>(`/admins/${id}`),
  setStatus: (id: number, status: number) => http.put<null>(`/admins/${id}/status`, { status }),
  resetPassword: (id: number, password: string) => http.put<null>(`/admins/${id}/password`, { password }),
  setRoles: (id: number, role_ids: number[]) => http.put<null>(`/admins/${id}/roles`, { role_ids }),
};
