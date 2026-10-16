import { http } from './client';
import { downloadCsv } from './download';
import type { ExportKind, ExportTask, Page } from './types';

export interface ExportQuery { page?: number; size?: number; kind?: string; status?: number }

export const exportApi = {
  list: (q: ExportQuery) => http.get<Page<ExportTask>>('/exports', q),
  /** 筛选条件与同步导出的 query 同名字段（`{}` = 全量），后端直接复用各自的取数结构 */
  create: (kind: ExportKind, params?: Record<string, unknown>) =>
    http.post<{ id: string; kind: string; status: number }>('/exports', { kind, params }),
  remove: (id: string) => http.del<{ deleted: number }>(`/exports/${id}`),
  /** 成品走 blob（要带 Bearer）；文件名用 kind 当前缀，和类型对得上就行 */
  download: (row: ExportTask) =>
    downloadCsv(`/exports/${row.id}/download`, undefined, row.kind),
};
