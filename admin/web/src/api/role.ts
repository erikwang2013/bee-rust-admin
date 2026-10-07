import { http } from './client';
import type { Page, Role } from './types';

export interface RoleQuery { page?: number; size?: number; name?: string; status?: number }
export interface RoleForm { name: string; code: string; sort: number; data_scope: number; status: number; remark: string }

export const roleApi = {
  list: (q: RoleQuery) => http.get<Page<Role>>('/roles', q),
  get: (id: string) => http.get<Role>(`/roles/${id}`),
  create: (data: RoleForm) => http.post<Role>('/roles', data),
  update: (id: string, data: RoleForm) => http.put<Role>(`/roles/${id}`, data),
  remove: (id: string) => http.del<null>(`/roles/${id}`),
  menus: (id: string) => http.get<string[]>(`/roles/${id}/menus`),
  setMenus: (id: string, menu_ids: string[]) => http.put<null>(`/roles/${id}/menus`, { menu_ids }),
  depts: (id: string) => http.get<string[]>(`/roles/${id}/depts`),
  setDepts: (id: string, dept_ids: string[]) => http.put<null>(`/roles/${id}/depts`, { dept_ids }),
};
