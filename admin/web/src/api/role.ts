import { http } from './client';
import type { Page, Role } from './types';

export interface RoleQuery { page?: number; size?: number; name?: string; status?: number }
export interface RoleForm { name: string; code: string; sort: number; data_scope: number; status: number; remark: string }

export const roleApi = {
  list: (q: RoleQuery) => http.get<Page<Role>>('/roles', q),
  get: (id: number) => http.get<Role>(`/roles/${id}`),
  create: (data: RoleForm) => http.post<Role>('/roles', data),
  update: (id: number, data: RoleForm) => http.put<Role>(`/roles/${id}`, data),
  remove: (id: number) => http.del<null>(`/roles/${id}`),
  menus: (id: number) => http.get<number[]>(`/roles/${id}/menus`),
  setMenus: (id: number, menu_ids: number[]) => http.put<null>(`/roles/${id}/menus`, { menu_ids }),
  depts: (id: number) => http.get<number[]>(`/roles/${id}/depts`),
  setDepts: (id: number, dept_ids: number[]) => http.put<null>(`/roles/${id}/depts`, { dept_ids }),
};
