import { http } from './client';
import { downloadCsv } from './download';
import type { ExportKind, ExportTask, Page } from './types';

export interface ExportQuery { page?: number; size?: number; kind?: string; status?: number }

/** 落盘文件名前缀：与后端 `kind.file_prefix` 一致（同步导出的 `audit-logs-<日期>.csv`
 * 也是这套）；认不出的 kind 原样用 code */
const FILE_PREFIX: Partial<Record<ExportKind, string>> = {
  'audit-log': 'audit-logs',
  'login-log': 'login-logs',
};

export const exportApi = {
  list: (q: ExportQuery) => http.get<Page<ExportTask>>('/exports', q),
  /** 筛选条件与同步导出的 query 同名字段（`{}` = 全量），后端直接复用各自的取数结构 */
  create: (kind: ExportKind, params?: Record<string, unknown>) =>
    http.post<{ id: string; kind: string; status: number }>('/exports', { kind, params }),
  /** 取消还没跑完的任务（后端在批间看到取消就收手，已写出的部分不留） */
  cancel: (id: string) => http.post<{ canceled: number }>(`/exports/${id}/cancel`),
  remove: (id: string) => http.del<{ deleted: number }>(`/exports/${id}`),
  /** 成品走 blob（要带 Bearer）；文件名前缀与后端 `file_prefix` 一致，和同步导出对得上 */
  download: (row: ExportTask) =>
    downloadCsv(`/exports/${row.id}/download`, undefined, FILE_PREFIX[row.kind as ExportKind] ?? row.kind),
};
