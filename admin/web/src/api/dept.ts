import { http } from './client';
import type { Dept } from './types';

export interface DeptForm { parent_id: number; name: string; sort: number; leader: string; phone: string; status: number }

export const deptApi = {
  tree: () => http.get<Dept[]>('/depts/tree'),
  create: (data: DeptForm) => http.post<Dept>('/depts', data),
  update: (id: number, data: DeptForm) => http.put<Dept>(`/depts/${id}`, data),
  remove: (id: number) => http.del<null>(`/depts/${id}`),
};
