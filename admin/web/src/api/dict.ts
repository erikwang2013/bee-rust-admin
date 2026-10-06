import { http } from './client';
import { downloadCsv } from './download';
import type { DictItem, DictType, Page } from './types';

export interface DictTypeQuery { page?: number; size?: number; name?: string; status?: number }
export interface DictTypeForm { name: string; code: string; status: number; remark: string }
export interface DictItemQuery {
  page?: number; size?: number; type_code?: string; label?: string; status?: number;
}
export interface DictItemForm {
  type_code: string; label: string; value: string; sort: number; status: number; remark: string;
}
/** 下拉数据源：只含启用项，后端已按 sort, id 排好序 */
export interface DictOption { label: string; value: string }

export const dictApi = {
  list: (q: DictTypeQuery) => http.get<Page<DictType>>('/dicts', q),
  create: (data: DictTypeForm) => http.post<DictType>('/dicts', data),
  /** code 是字典项的关联键，不可改 —— 更新体里不带它 */
  update: (id: number, data: Omit<DictTypeForm, 'code'>) => http.put<DictType>(`/dicts/${id}`, data),
  remove: (id: number) => http.del<null>(`/dicts/${id}`),

  items: (code: string) => http.get<DictOption[]>(`/dicts/${code}/items`),
  itemList: (q: DictItemQuery) => http.get<Page<DictItem>>('/dict-items', q),
  itemCreate: (data: DictItemForm) => http.post<DictItem>('/dict-items', data),
  itemUpdate: (id: number, data: DictItemForm) => http.put<DictItem>(`/dict-items/${id}`, data),
  itemRemove: (id: number) => http.del<null>(`/dict-items/${id}`),
  itemExport: (q: DictItemQuery) => downloadCsv('/dict-items/export', q, 'dict-items'),
};
