import { http } from './client';
import { downloadCsv } from './download';
import type { ExportKind, ExportTask, Page } from './types';

export interface ExportQuery { page?: number; size?: number; kind?: string; status?: number }

export const exportApi = {
  list: (q: ExportQuery) => http.get<Page<ExportTask>>('/exports', q),
  /** params 省略 = 全量：带条件的导出在各自列表页同步下就行（导出中心不做筛选表单） */
  create: (kind: ExportKind) =>
    http.post<{ id: string; kind: string; status: number }>('/exports', { kind }),
  remove: (id: string) => http.del<{ deleted: number }>(`/exports/${id}`),
  /** 成品走 blob（要带 Bearer）；文件名用 kind 当前缀，和类型对得上就行 */
  download: (row: ExportTask) =>
    downloadCsv(`/exports/${row.id}/download`, undefined, row.kind),
};
