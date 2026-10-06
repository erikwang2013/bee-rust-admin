import { http } from './client';
import type { AuditLog, Page } from './types';

export interface AuditLogQuery {
  page?: number; size?: number; username?: string; module?: string;
  status?: number; start?: string; end?: string;
}

export const auditLogApi = {
  list: (q: AuditLogQuery) => http.get<Page<AuditLog>>('/audit-logs', q),
  clear: (q: AuditLogQuery) => http.del<{ deleted: number }>('/audit-logs', q),
};
