import { http } from './client';
import type { Menu } from './types';

export interface MenuForm {
  /** 顶级用 `''`（后端把空串当 0）；树里根节点回显的是后端 enc(0) 的短串，编辑时归一 */
  parent_id: string; name: string; type: 'M' | 'C' | 'F'; perm: string; path: string;
  component: string; icon: string; sort: number; visible: number; status: number;
}

export const menuApi = {
  tree: () => http.get<Menu[]>('/menus/tree'),
  create: (data: MenuForm) => http.post<Menu>('/menus', data),
  update: (id: string, data: MenuForm) => http.put<Menu>(`/menus/${id}`, data),
  remove: (id: string) => http.del<null>(`/menus/${id}`),
};
