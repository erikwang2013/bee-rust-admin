import { http } from './client';
import type { ImportKind, ImportReport } from './types';

export const importApi = {
  /**
   * 上传 CSV 导入（表单字段名 `file` 与后端逐字一致）。
   * 种类走 query：后端按种类复用各模块的 `:add` 权限码。
   */
  upload: (kind: ImportKind, file: File) => {
    const form = new FormData();
    form.append('file', file);
    // 逐行校验可能超过全局 15s；导入专用长超时，别让大 CSV 传到一半被前端掐断
    return http.post<ImportReport>(`/imports?kind=${kind}`, form, { timeout: 120000 });
  },
};
