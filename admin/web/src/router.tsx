import { lazy, Suspense, type ReactNode } from 'react';
import { createBrowserRouter, Navigate } from 'react-router';
import { Spin } from 'antd';
import RequireAuth, { RequirePerm } from './router/guard';
import BasicLayout from './layouts/BasicLayout';
import LoginPage from './pages/login/index';
import Forbidden from './pages/error/403';
import NotFound from './pages/error/404';

const Dashboard = lazy(() => import('./pages/dashboard/index'));
const ProfilePage = lazy(() => import('./pages/profile/index'));
const AdminPage = lazy(() => import('./pages/system/admin/index'));
const RolePage = lazy(() => import('./pages/system/role/index'));
const MenuPage = lazy(() => import('./pages/system/menu/index'));
const DeptPage = lazy(() => import('./pages/system/dept/index'));
const LoginLogPage = lazy(() => import('./pages/system/loginlog/index'));
const AuditLogPage = lazy(() => import('./pages/system/auditlog/index'));

const loading = (
  <div style={{ display: 'flex', justifyContent: 'center', paddingTop: 120 }}>
    <Spin size="large" />
  </div>
);

function L({ children }: { children: ReactNode }) {
  return <Suspense fallback={loading}>{children}</Suspense>;
}

export const router = createBrowserRouter([
  { path: '/login', element: <LoginPage /> },
  {
    path: '/',
    element: <RequireAuth />,
    children: [
      {
        element: <BasicLayout />,
        children: [
          { index: true, element: <Navigate to="/dashboard" replace /> },
          { path: 'dashboard', element: <L><Dashboard /></L> },
          { path: 'system/admin', element: <RequirePerm code="system:admin:list"><L><AdminPage /></L></RequirePerm> },
          { path: 'system/role', element: <RequirePerm code="system:role:list"><L><RolePage /></L></RequirePerm> },
          { path: 'system/menu', element: <RequirePerm code="system:menu:list"><L><MenuPage /></L></RequirePerm> },
          { path: 'system/dept', element: <RequirePerm code="system:dept:list"><L><DeptPage /></L></RequirePerm> },
          { path: 'system/login-log', element: <RequirePerm code="system:loginlog:list"><L><LoginLogPage /></L></RequirePerm> },
          { path: 'system/audit-log', element: <RequirePerm code="system:auditlog:list"><L><AuditLogPage /></L></RequirePerm> },
          { path: 'profile', element: <L><ProfilePage /></L> },
          { path: '403', element: <Forbidden /> },
          { path: '*', element: <NotFound /> },
        ],
      },
    ],
  },
]);
