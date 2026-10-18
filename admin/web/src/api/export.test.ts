import { beforeEach, describe, expect, it, vi } from 'vitest';
import { exportApi } from './export';
import type { ExportTask } from './types';

// 只测下载前缀的拼装：HTTP 与落盘各自有主，这里整体替掉
const { downloadCsv } = vi.hoisted(() => ({ downloadCsv: vi.fn() }));
vi.mock('./client', () => ({ http: {} }));
vi.mock('./download', () => ({ downloadCsv }));

const row = (id: string, kind: string) => ({ id, kind } as unknown as ExportTask);

beforeEach(() => downloadCsv.mockClear());

describe('exportApi.download', () => {
  it('文件名前缀用后端 kind.file_prefix，而不是 kind 原值', () => {
    exportApi.download(row('AbC123', 'login-log'));
    expect(downloadCsv).toHaveBeenCalledWith('/exports/AbC123/download', undefined, 'login-logs');
    exportApi.download(row('XyZ789', 'audit-log'));
    expect(downloadCsv).toHaveBeenLastCalledWith('/exports/XyZ789/download', undefined, 'audit-logs');
  });

  it('认不出的 kind 原样用 code（与后端注册表外的兜底一致）', () => {
    exportApi.download(row('Q', 'other'));
    expect(downloadCsv).toHaveBeenLastCalledWith('/exports/Q/download', undefined, 'other');
  });
});
