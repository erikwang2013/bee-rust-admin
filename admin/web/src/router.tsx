import { lazy, Suspense, type ReactNode } from 'react';
import { createBrowserRouter, Navigate } from 'react-router';
import { Spin } from 'antd';
import RequireAuth from './router/guard';
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
          { path: 'system/admin', element: <L><AdminPage /></L> },
          { path: 'system/role', element: <L><RolePage /></L> },
          { path: 'system/menu', element: <L><MenuPage /></L> },
          { path: 'system/dept', element: <L><DeptPage /></L> },
          { path: 'profile', element: <L><ProfilePage /></L> },
          { path: '403', element: <Forbidden /> },
          { path: '*', element: <NotFound /> },
        ],
      },
    ],
  },
]);
