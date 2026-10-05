import type { ReactNode } from 'react';
import { useAuth } from './AuthContext';

/** 按钮级权限：无权限时不渲染 children。后端仍会逐接口鉴权，这里只管显示。 */
export default function Auth({ code, children }: { code: string; children: ReactNode }) {
  const { has } = useAuth();
  return has(code) ? <>{children}</> : null;
}
