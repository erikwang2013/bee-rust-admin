import { http } from './client';
import type { Dept } from './types';

/** 顶级用 `''`（后端把空串当 0）；树里根节点回显的是后端 enc(0) 的短串，编辑时归一 */
export interface DeptForm { parent_id: string; name: string; sort: number; leader: string; phone: string; status: number }

export const deptApi = {
  tree: () => http.get<Dept[]>('/depts/tree'),
  create: (data: DeptForm) => http.post<Dept>('/depts', data),
  update: (id: string, data: DeptForm) => http.put<Dept>(`/depts/${id}`, data),
  remove: (id: string) => http.del<null>(`/depts/${id}`),
};
