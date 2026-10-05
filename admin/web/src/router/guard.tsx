import type { ReactNode } from 'react';
import { Spin } from 'antd';
import { Navigate, Outlet, useLocation } from 'react-router';
import { useAuth } from '../auth/AuthContext';

/** 未登录跳 /login；token 有效但 profile 未就绪时显示加载。 */
export default function RequireAuth() {
  const { user, ready } = useAuth();
  const loc = useLocation();

  if (!ready) {
    return (
      <div style={{ display: 'flex', justifyContent: 'center', alignItems: 'center', height: '100vh' }}>
        <Spin size="large" />
      </div>
    );
  }
  if (!user) {
    return <Navigate to="/login" state={{ from: loc.pathname }} replace />;
  }
  return <Outlet />;
}

/** 路由级权限：无权限码时跳 /403（后端仍逐接口鉴权，这里只管显示）。 */
export function RequirePerm({ code, children }: { code: string; children: ReactNode }) {
  const { has } = useAuth();
  return has(code) ? <>{children}</> : <Navigate to="/403" replace />;
}
