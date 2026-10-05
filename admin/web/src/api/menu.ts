import { http } from './client';
import type { Menu } from './types';

export interface MenuForm {
  parent_id: number; name: string; type: 'M' | 'C' | 'F'; perm: string; path: string;
  component: string; icon: string; sort: number; visible: number; status: number;
}

export const menuApi = {
  tree: () => http.get<Menu[]>('/menus/tree'),
  create: (data: MenuForm) => http.post<Menu>('/menus', data),
  update: (id: number, data: MenuForm) => http.put<Menu>(`/menus/${id}`, data),
  remove: (id: number) => http.del<null>(`/menus/${id}`),
};
