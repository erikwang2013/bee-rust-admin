import { http } from './client';
import type { Page, PublicConfig, SysConfig } from './types';

// 公开配置的类型就住在这个模块的场景里（fetchPublicConfig 的返回值），从 types 再导出一次
export type { PublicConfig } from './types';

export interface ConfigQuery { group?: string }
export interface ConfigForm {
  key: string; value: string; value_type: string; group: string; is_public: number; remark: string;
}
/** 部分更新：只提交改动过的字段 */
export interface ConfigUpdateForm {
  value?: string; value_type?: string; is_public?: number; remark?: string;
}

export const configApi = {
  list: (q?: ConfigQuery) => http.get<Page<SysConfig>>('/configs', q),
  create: (data: ConfigForm) => http.post<{ id: string }>('/configs', data),
  update: (id: string, data: ConfigUpdateForm) => http.put<null>(`/configs/${id}`, data),
  remove: (id: string) => http.del<null>(`/configs/${id}`),
};

/**
 * 公开配置（免登录）。登录页副标题与 SiteInfo 页脚都会读：同一次页面加载里两个组件
 * 并发挂载，共享同一个 in-flight 请求；resolve 后即释放，**不做缓存** ——
 * 验收要求「改配置 → 登录页/页脚刷新即变」。
 */
let inflight: Promise<PublicConfig> | null = null;
export function fetchPublicConfig(): Promise<PublicConfig> {
  if (!inflight) {
    inflight = http
      .get<PublicConfig>('/configs/public')
      .finally(() => { inflight = null; });
  }
  return inflight;
}
