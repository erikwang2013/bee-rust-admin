# 管理后台前端（M5）实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 `admin/web/` 交付管理后台 SPA：登录、动态菜单布局、管理员/角色/菜单/部门/登录记录五个管理页、按钮级权限、个人中心。

**Architecture:** Vite + React 18 + TS + antd 5 单页应用。后端契约见设计文档 §5.3（本计划 F2 把 JSON 形状钉死）。鉴权用 JWT：token 存 localStorage，axios 拦截器注入 `Authorization: Bearer`，401 回登录页。菜单/权限由后端 `/auth/menus`、`/auth/profile` 驱动，前端只做显示控制（后端仍逐接口鉴权）。

**Tech Stack:** Vite 7、React 18.3、TypeScript 5、antd 5、@ant-design/icons、react-router 7、axios、dayjs（antd 自带）、pnpm。

**设计依据:** `docs/superpowers/specs/2026-10-05-bee-rust-admin-design.md` §6、§5.3。

**说明（验证边界）:** 本计划期间后端 M3/M4 可能尚未就绪，因此每个任务的验收 = `pnpm build`（tsc 类型检查 + vite 打包）通过 + 关键页面代码审查；真实端到端点击验证在 M6（后端起来后）做，不在这里假装通过。

**React 版本说明:** 设计文档写 React 19，本计划用 React 18.3——antd 5 对 React 19 需要额外补丁包，18 是零补丁的最稳组合，对本项目没有任何功能损失。

---

## 接口契约（前后端共用的钉子，M3 必须一致）

统一信封：`{ code: number, msg: string, data: T }`，`code === 0` 成功；HTTP 401 = token 失效，403 = 无权限。

```
POST /api/v1/auth/login          { username, password } → { token, expires_in, user }
POST /api/v1/auth/logout         → null
GET  /api/v1/auth/profile        → { user, roles: string[], perms: string[] }
GET  /api/v1/auth/menus          → MenuNode[]（只有目录/菜单，按钮不返）
PUT  /api/v1/auth/password       { old_password, new_password } → null

GET    /api/v1/admins?page&size&username&status&dept_id → Page<Admin>
POST   /api/v1/admins            { username, password, nickname, email, phone, sex, dept_id, status, remark, role_ids }
GET    /api/v1/admins/{id}       → Admin
PUT    /api/v1/admins/{id}       { nickname, email, phone, sex, dept_id, status, remark, role_ids }
DELETE /api/v1/admins/{id}       → null
PUT    /api/v1/admins/{id}/status   { status }
PUT    /api/v1/admins/{id}/password { password }
PUT    /api/v1/admins/{id}/roles    { role_ids }

GET    /api/v1/roles?page&size&name&status → Page<Role>
POST   /api/v1/roles / PUT /api/v1/roles/{id} / DELETE /api/v1/roles/{id} / GET /api/v1/roles/{id}
GET    /api/v1/roles/{id}/menus  → number[]（已勾选 menu_id）
PUT    /api/v1/roles/{id}/menus  { menu_ids: number[] }
GET    /api/v1/roles/{id}/depts  → number[]
PUT    /api/v1/roles/{id}/depts  { dept_ids: number[] }

GET    /api/v1/menus/tree        → Menu[]（含按钮，管理页面用）
POST   /api/v1/menus / PUT /api/v1/menus/{id} / DELETE /api/v1/menus/{id}
GET    /api/v1/depts/tree        → Dept[]
POST   /api/v1/depts / PUT /api/v1/depts/{id} / DELETE /api/v1/depts/{id}

GET    /api/v1/login-logs?page&size&username&status&start&end → Page<LoginLog>
DELETE /api/v1/login-logs?username=&status=&start=&end= → null
```

type 定义（`src/api/types.ts` 落地）：

```ts
export interface ApiResult<T> { code: number; msg: string; data: T }
export interface Page<T> { list: T[]; total: number }

export interface UserInfo { id: number; username: string; nickname: string; avatar: string; is_super: boolean; dept_id: number }
export interface Profile { user: UserInfo; roles: string[]; perms: string[] }
export interface MenuNode { id: number; parent_id: number; name: string; path: string; icon: string; children?: MenuNode[] }

export interface Admin {
  id: number; username: string; nickname: string; email: string; phone: string; sex: number;
  dept_id: number; dept_name?: string; status: number; is_super: boolean; remark: string;
  created_at: string; role_ids?: number[]; role_names?: string[];
}
export interface Role {
  id: number; name: string; code: string; sort: number; data_scope: number;
  status: number; remark: string; created_at: string;
}
export interface Menu {
  id: number; parent_id: number; name: string; type: 'M' | 'C' | 'F'; perm: string;
  path: string; component: string; icon: string; sort: number; visible: number;
  status: number; children?: Menu[];
}
export interface Dept {
  id: number; parent_id: number; name: string; sort: number; leader: string;
  phone: string; status: number; children?: Dept[];
}
export interface LoginLog {
  id: number; username: string; ip: string; user_agent: string;
  status: number; msg: string; created_at: string;
}
export const DATA_SCOPE_LABELS: Record<number, string> = {
  1: '全部数据', 2: '本部门及以下', 3: '本部门', 4: '仅本人', 5: '自定义',
};
```

---

## 文件结构

```
admin/web/
  package.json  tsconfig.json  tsconfig.node.json  vite.config.ts  index.html  .gitignore
  src/main.tsx                  ConfigProvider(zhCN) + AntdApp + RouterProvider
  src/router.tsx                路由表（懒加载 + 守卫包装）
  src/api/types.ts              契约类型（上面已全部给出）
  src/api/client.ts             axios 实例 + 拦截器 + 类型化 http.get/post/put/delete
  src/api/auth.ts admin.ts role.ts menu.ts dept.ts loginLog.ts
  src/auth/AuthContext.tsx      user/perms/menus + login/logout/has()
  src/auth/Auth.tsx             <Auth code="system:admin:add">
  src/router/guard.tsx          RequireAuth（未登录跳 /login，首次进入拉 profile）
  src/layouts/BasicLayout.tsx   Sider 动态菜单 + 面包屑 + 顶栏用户下拉
  src/icons.ts                  图标名字符串 → antd 图标组件
  src/pages/login/index.tsx
  src/pages/dashboard/index.tsx
  src/pages/profile/index.tsx
  src/pages/system/admin/index.tsx
  src/pages/system/role/index.tsx
  src/pages/system/menu/index.tsx
  src/pages/system/dept/index.tsx
  src/pages/system/loginlog/index.tsx
  src/pages/error/403.tsx  404.tsx
```

---

### Task F1: 脚手架 + 路由骨架

**Files:**
- Create: `admin/web/package.json`, `vite.config.ts`, `tsconfig.json`, `tsconfig.node.json`, `index.html`, `.gitignore`, `src/main.tsx`, `src/router.tsx`, `src/pages/error/403.tsx`, `src/pages/error/404.tsx`

- [ ] **Step 1: 写 `package.json`**

```json
{
  "name": "bee-admin-web",
  "private": true,
  "version": "0.1.0",
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "tsc -b && vite build",
    "preview": "vite preview"
  },
  "dependencies": {
    "@ant-design/icons": "^5.6.1",
    "antd": "^5.24.0",
    "axios": "^1.8.0",
    "dayjs": "^1.11.13",
    "react": "^18.3.1",
    "react-dom": "^18.3.1",
    "react-router": "^7.3.0"
  },
  "devDependencies": {
    "@types/react": "^18.3.18",
    "@types/react-dom": "^18.3.5",
    "@vitejs/plugin-react": "^4.3.4",
    "typescript": "~5.7.3",
    "vite": "^6.2.0"
  }
}
```

（`react-router@7` 的 `react-router` 包即含 `BrowserRouter/RouterProvider`；不需要 `react-router-dom`。）

- [ ] **Step 2: 写 `vite.config.ts`（dev 代理到后端）**

```ts
import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    proxy: {
      '/api': { target: 'http://127.0.0.1:8080', changeOrigin: true },
    },
  },
  build: { outDir: 'dist', sourcemap: false },
});
```

- [ ] **Step 3: 写 `tsconfig.json` / `tsconfig.node.json` / `index.html` / `.gitignore`**

`tsconfig.json`：

```json
{
  "compilerOptions": {
    "target": "ES2020",
    "useDefineForClassFields": true,
    "lib": ["ES2020", "DOM", "DOM.Iterable"],
    "module": "ESNext",
    "skipLibCheck": true,
    "moduleResolution": "bundler",
    "allowImportingTsExtensions": true,
    "resolveJsonModule": true,
    "isolatedModules": true,
    "noEmit": true,
    "jsx": "react-jsx",
    "strict": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "noFallthroughCasesInSwitch": true
  },
  "include": ["src"]
}
```

`tsconfig.node.json`：

```json
{
  "compilerOptions": {
    "composite": true,
    "skipLibCheck": true,
    "module": "ESNext",
    "moduleResolution": "bundler",
    "allowSyntheticDefaultImports": true,
    "strict": true,
    "noEmit": true
  },
  "include": ["vite.config.ts"]
}
```

`index.html`：

```html
<!doctype html>
<html lang="zh-CN">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>bee-rust 管理后台</title>
  </head>
  <body>
    <div id="root"></div>
    <script type="module" src="/src/main.tsx"></script>
  </body>
</html>
```

`.gitignore`：

```
node_modules
dist
*.local
```

- [ ] **Step 4: 写 `src/main.tsx` 与 `src/router.tsx`（先只有 403/404 与占位首页）**

`src/main.tsx`：

```tsx
import React from 'react';
import ReactDOM from 'react-dom/client';
import { ConfigProvider, App as AntdApp } from 'antd';
import zhCN from 'antd/locale/zh_CN';
import 'dayjs/locale/zh-cn';
import { RouterProvider } from 'react-router';
import { router } from './router';

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <ConfigProvider locale={zhCN}>
      <AntdApp>
        <RouterProvider router={router} />
      </AntdApp>
    </ConfigProvider>
  </React.StrictMode>,
);
```

`src/router.tsx`（本任务先建最小路由；F3/F4 会替换为带守卫与布局的完整表）：

```tsx
import { createBrowserRouter } from 'react-router';
import NotFound from './pages/error/404';
import Forbidden from './pages/error/403';

export const router = createBrowserRouter([
  { path: '/403', element: <Forbidden /> },
  { path: '*', element: <NotFound /> },
]);
```

`src/pages/error/403.tsx`：

```tsx
import { Result, Button } from 'antd';
import { useNavigate } from 'react-router';

export default function Forbidden() {
  const nav = useNavigate();
  return (
    <Result
      status="403"
      title="403"
      subTitle="没有权限访问该页面"
      extra={<Button type="primary" onClick={() => nav('/')}>返回首页</Button>}
    />
  );
}
```

`src/pages/error/404.tsx`：

```tsx
import { Result, Button } from 'antd';
import { useNavigate } from 'react-router';

export default function NotFound() {
  const nav = useNavigate();
  return (
    <Result
      status="404"
      title="404"
      subTitle="页面不存在"
      extra={<Button type="primary" onClick={() => nav('/')}>返回首页</Button>}
    />
  );
}
```

- [ ] **Step 5: 安装依赖并验证构建**

Run: `cd admin/web && pnpm install && pnpm build`
Expected: `tsc -b` 无错误，`vite build` 产出 `dist/`（最后一行类似 `✓ built in Xs`）。

- [ ] **Step 6: Commit**

```bash
git add admin/web
git commit -m "feat(admin-web): Vite+React+antd 脚手架与路由骨架"
```

---

### Task F2: API 层（契约类型 + axios 客户端 + 各模块请求）

**Files:**
- Create: `admin/web/src/api/types.ts`（内容 = 上面「接口契约」里给出的完整 `types.ts`，照抄）
- Create: `admin/web/src/api/client.ts`
- Create: `admin/web/src/api/auth.ts`, `admin.ts`, `role.ts`, `menu.ts`, `dept.ts`, `loginLog.ts`

- [ ] **Step 1: 写 `src/api/types.ts`**

照抄上文「接口契约」小节中的 `types.ts` 代码块全文（含 `DATA_SCOPE_LABELS`）。

- [ ] **Step 2: 写 `src/api/client.ts`**

```ts
import axios, { type AxiosRequestConfig } from 'axios';
import { message } from 'antd';
import type { ApiResult } from './types';

export const TOKEN_KEY = 'bee_admin_token';

const client = axios.create({ baseURL: '/api/v1', timeout: 15000 });

client.interceptors.request.use((cfg) => {
  const token = localStorage.getItem(TOKEN_KEY);
  if (token) cfg.headers.Authorization = `Bearer ${token}`;
  return cfg;
});

client.interceptors.response.use(
  (res) => {
    const body = res.data as ApiResult<unknown>;
    if (body && typeof body.code === 'number' && body.code !== 0) {
      message.error(body.msg || '请求失败');
      return Promise.reject(new Error(body.msg || 'request failed'));
    }
    return res;
  },
  (err) => {
    const status = err.response?.status;
    if (status === 401) {
      localStorage.removeItem(TOKEN_KEY);
      if (!window.location.pathname.startsWith('/login')) {
        window.location.href = '/login';
      }
    } else if (status === 403) {
      message.error(err.response?.data?.msg || '没有权限');
    } else {
      message.error(err.response?.data?.msg || err.message || '网络错误');
    }
    return Promise.reject(err);
  },
);

async function unwrap<T>(p: Promise<{ data: ApiResult<T> }>): Promise<T> {
  const res = await p;
  return res.data.data;
}

export const http = {
  get: <T>(url: string, params?: object, cfg?: AxiosRequestConfig) =>
    unwrap<T>(client.get(url, { params, ...cfg })),
  post: <T>(url: string, data?: object) => unwrap<T>(client.post(url, data)),
  put: <T>(url: string, data?: object) => unwrap<T>(client.put(url, data)),
  del: <T>(url: string, params?: object) => unwrap<T>(client.delete(url, { params })),
};
```

- [ ] **Step 3: 写六个 api 模块**

`src/api/auth.ts`：

```ts
import { http } from './client';
import type { MenuNode, Profile, UserInfo } from './types';

export interface LoginResult { token: string; expires_in: number; user: UserInfo }

export const authApi = {
  login: (username: string, password: string) =>
    http.post<LoginResult>('/auth/login', { username, password }),
  logout: () => http.post<null>('/auth/logout'),
  profile: () => http.get<Profile>('/auth/profile'),
  menus: () => http.get<MenuNode[]>('/auth/menus'),
  changePassword: (old_password: string, new_password: string) =>
    http.put<null>('/auth/password', { old_password, new_password }),
};
```

`src/api/admin.ts`：

```ts
import { http } from './client';
import type { Admin, Page } from './types';

export interface AdminQuery { page?: number; size?: number; username?: string; status?: number; dept_id?: number }
export interface AdminForm {
  username?: string; password?: string; nickname: string; email: string; phone: string;
  sex: number; dept_id: number; status: number; remark: string; role_ids: number[];
}

export const adminApi = {
  list: (q: AdminQuery) => http.get<Page<Admin>>('/admins', q),
  get: (id: number) => http.get<Admin>(`/admins/${id}`),
  create: (data: AdminForm) => http.post<Admin>('/admins', data),
  update: (id: number, data: AdminForm) => http.put<Admin>(`/admins/${id}`, data),
  remove: (id: number) => http.del<null>(`/admins/${id}`),
  setStatus: (id: number, status: number) => http.put<null>(`/admins/${id}/status`, { status }),
  resetPassword: (id: number, password: string) => http.put<null>(`/admins/${id}/password`, { password }),
  setRoles: (id: number, role_ids: number[]) => http.put<null>(`/admins/${id}/roles`, { role_ids }),
};
```

`src/api/role.ts`：

```ts
import { http } from './client';
import type { Page, Role } from './types';

export interface RoleQuery { page?: number; size?: number; name?: string; status?: number }
export interface RoleForm { name: string; code: string; sort: number; data_scope: number; status: number; remark: string }

export const roleApi = {
  list: (q: RoleQuery) => http.get<Page<Role>>('/roles', q),
  get: (id: number) => http.get<Role>(`/roles/${id}`),
  create: (data: RoleForm) => http.post<Role>('/roles', data),
  update: (id: number, data: RoleForm) => http.put<Role>(`/roles/${id}`, data),
  remove: (id: number) => http.del<null>(`/roles/${id}`),
  menus: (id: number) => http.get<number[]>(`/roles/${id}/menus`),
  setMenus: (id: number, menu_ids: number[]) => http.put<null>(`/roles/${id}/menus`, { menu_ids }),
  depts: (id: number) => http.get<number[]>(`/roles/${id}/depts`),
  setDepts: (id: number, dept_ids: number[]) => http.put<null>(`/roles/${id}/depts`, { dept_ids }),
};
```

`src/api/menu.ts`：

```ts
import { http } from './client';
import type { Menu } from './types';

export interface MenuForm {
  parent_id: number; name: string; type: 'M' | 'C' | 'F'; perm: string; path: string;
  component: string; icon: string; sort: number; visible: number; status: number;
}

export const menuApi = {
  tree: () => http.get<Menu[]>('/menus/tree'),
  create: (data: MenuForm) => http.post<Menu>('/menus', data),
  update: (id: number, data: MenuForm) => http.put<Menu>(`/menus/${id}`, data),
  remove: (id: number) => http.del<null>(`/menus/${id}`),
};
```

`src/api/dept.ts`：

```ts
import { http } from './client';
import type { Dept } from './types';

export interface DeptForm { parent_id: number; name: string; sort: number; leader: string; phone: string; status: number }

export const deptApi = {
  tree: () => http.get<Dept[]>('/depts/tree'),
  create: (data: DeptForm) => http.post<Dept>('/depts', data),
  update: (id: number, data: DeptForm) => http.put<Dept>(`/depts/${id}`, data),
  remove: (id: number) => http.del<null>(`/depts/${id}`),
};
```

`src/api/loginLog.ts`：

```ts
import { http } from './client';
import type { LoginLog, Page } from './types';

export interface LoginLogQuery {
  page?: number; size?: number; username?: string; status?: number; start?: string; end?: string;
}

export const loginLogApi = {
  list: (q: LoginLogQuery) => http.get<Page<LoginLog>>('/login-logs', q),
  clear: (q: LoginLogQuery) => http.del<null>('/login-logs', q),
};
```

- [ ] **Step 4: 验证构建**

Run: `cd admin/web && pnpm build`
Expected: 通过。（此时各 api 模块还没被页面引用，`tsc` 不会因未使用而报错；`noUnusedLocals` 只作用于局部变量。）

- [ ] **Step 5: Commit**

```bash
git add admin/web/src/api
git commit -m "feat(admin-web): API 层（契约类型 + axios 拦截器 + 各模块请求）"
```

---

### Task F3: 鉴权上下文 + 路由守卫 + 登录页

**Files:**
- Create: `admin/web/src/auth/AuthContext.tsx`, `src/auth/Auth.tsx`, `src/router/guard.tsx`
- Create: `admin/web/src/pages/login/index.tsx`
- Modify: `admin/web/src/router.tsx`（换成完整路由表，首页先用占位）

- [ ] **Step 1: 写 `src/auth/AuthContext.tsx`**

```tsx
import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from 'react';
import { authApi } from '../api/auth';
import { TOKEN_KEY } from '../api/client';
import type { MenuNode, UserInfo } from '../api/types';

interface AuthState {
  user: UserInfo | null;
  perms: string[];
  menus: MenuNode[];
  ready: boolean;
  login: (username: string, password: string) => Promise<void>;
  logout: () => Promise<void>;
  reload: () => Promise<void>;
  has: (code: string) => boolean;
}

const AuthContext = createContext<AuthState | null>(null);

export function AuthProvider({ children }: { children: ReactNode }) {
  const [user, setUser] = useState<UserInfo | null>(null);
  const [perms, setPerms] = useState<string[]>([]);
  const [menus, setMenus] = useState<MenuNode[]>([]);
  const [ready, setReady] = useState(false);

  const reload = useCallback(async () => {
    if (!localStorage.getItem(TOKEN_KEY)) {
      setUser(null); setPerms([]); setMenus([]); setReady(true);
      return;
    }
    try {
      const profile = await authApi.profile();
      const tree = await authApi.menus();
      setUser(profile.user);
      setPerms(profile.perms);
      setMenus(tree);
    } catch {
      localStorage.removeItem(TOKEN_KEY);
      setUser(null);
    } finally {
      setReady(true);
    }
  }, []);

  useEffect(() => { void reload(); }, [reload]);

  const login = useCallback(async (username: string, password: string) => {
    const res = await authApi.login(username, password);
    localStorage.setItem(TOKEN_KEY, res.token);
    const profile = await authApi.profile();
    const tree = await authApi.menus();
    setUser(profile.user);
    setPerms(profile.perms);
    setMenus(tree);
    setReady(true);
  }, []);

  const logout = useCallback(async () => {
    try { await authApi.logout(); } catch { /* token 已失效也要能退出 */ }
    localStorage.removeItem(TOKEN_KEY);
    setUser(null); setPerms([]); setMenus([]);
  }, []);

  const has = useCallback(
    (code: string) => perms.includes('*:*:*') || perms.includes(code),
    [perms],
  );

  const value = useMemo(
    () => ({ user, perms, menus, ready, login, logout, reload, has }),
    [user, perms, menus, ready, login, logout, reload, has],
  );

  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>;
}

export function useAuth(): AuthState {
  const ctx = useContext(AuthContext);
  if (!ctx) throw new Error('useAuth 必须在 AuthProvider 内使用');
  return ctx;
}
```

- [ ] **Step 2: 写 `src/auth/Auth.tsx`**

```tsx
import type { ReactNode } from 'react';
import { useAuth } from './AuthContext';

/** 按钮级权限：无权限时不渲染 children。后端仍会逐接口鉴权，这里只管显示。 */
export default function Auth({ code, children }: { code: string; children: ReactNode }) {
  const { has } = useAuth();
  return has(code) ? <>{children}</> : null;
}
```

- [ ] **Step 3: 写 `src/router/guard.tsx`**

```tsx
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
```

- [ ] **Step 4: 写 `src/pages/login/index.tsx`**

```tsx
import { useState } from 'react';
import { App, Button, Card, Form, Input, Typography } from 'antd';
import { LockOutlined, UserOutlined } from '@ant-design/icons';
import { useNavigate } from 'react-router';
import { useAuth } from '../../auth/AuthContext';

export default function LoginPage() {
  const { login } = useAuth();
  const { message } = App.useApp();
  const nav = useNavigate();
  const [loading, setLoading] = useState(false);

  const onFinish = async (v: { username: string; password: string }) => {
    setLoading(true);
    try {
      await login(v.username, v.password);
      nav('/', { replace: true });
    } catch {
      // 具体错误消息由 axios 拦截器提示
    } finally {
      setLoading(false);
    }
  };

  return (
    <div style={{ height: '100vh', display: 'flex', alignItems: 'center', justifyContent: 'center', background: '#f0f2f5' }}>
      <Card style={{ width: 380 }}>
        <Typography.Title level={3} style={{ textAlign: 'center', marginBottom: 24 }}>
          bee-rust 管理后台
        </Typography.Title>
        <Form onFinish={onFinish} size="large">
          <Form.Item name="username" rules={[{ required: true, message: '请输入用户名' }]}>
            <Input prefix={<UserOutlined />} placeholder="用户名" autoComplete="username" />
          </Form.Item>
          <Form.Item name="password" rules={[{ required: true, message: '请输入密码' }]}>
            <Input.Password prefix={<LockOutlined />} placeholder="密码" autoComplete="current-password" />
          </Form.Item>
          <Form.Item>
            <Button type="primary" htmlType="submit" block loading={loading}>
              登录
            </Button>
          </Form.Item>
        </Form>
      </Card>
    </div>
  );
}
```

（用 `App.useApp()` 取 message，配合 `main.tsx` 里已有的 `<AntdApp>`，避免 antd 静态方法与主题上下文脱节。）

- [ ] **Step 5: 改 `src/router.tsx`（完整路由表；布局用占位，F4 换成 BasicLayout）**

```tsx
import { lazy, Suspense } from 'react';
import { createBrowserRouter, Navigate } from 'react-router';
import { Spin } from 'antd';
import { AuthProvider } from './auth/AuthContext';
import RequireAuth from './router/guard';
import LoginPage from './pages/login/index';
import Forbidden from './pages/error/403';
import NotFound from './pages/error/404';

const Dashboard = lazy(() => import('./pages/dashboard/index'));
const AdminPage = lazy(() => import('./pages/system/admin/index'));
const RolePage = lazy(() => import('./pages/system/role/index'));
const MenuPage = lazy(() => import('./pages/system/menu/index'));
const DeptPage = lazy(() => import('./pages/system/dept/index'));
const LoginLogPage = lazy(() => import('./pages/system/loginlog/index'));
const ProfilePage = lazy(() => import('./pages/profile/index'));

const loading = (
  <div style={{ display: 'flex', justifyContent: 'center', paddingTop: 120 }}>
    <Spin size="large" />
  </div>
);

function L({ children }: { children: React.ReactNode }) {
  return <Suspense fallback={loading}>{children}</Suspense>;
}

export const router = createBrowserRouter([
  { path: '/login', element: <LoginPage /> },
  {
    path: '/',
    element: (
      <AuthProvider>
        <RequireAuth />
      </AuthProvider>
    ),
    children: [
      {
        element: <div />, // F4 换成 BasicLayout
        children: [
          { index: true, element: <Navigate to="/dashboard" replace /> },
          { path: 'dashboard', element: <L><Dashboard /></L> },
          { path: 'system/admin', element: <L><AdminPage /></L> },
          { path: 'system/role', element: <L><RolePage /></L> },
          { path: 'system/menu', element: <L><MenuPage /></L> },
          { path: 'system/dept', element: <L><DeptPage /></L> },
          { path: 'system/login-log', element: <L><LoginLogPage /></L> },
          { path: 'profile', element: <L><ProfilePage /></L> },
          { path: '403', element: <Forbidden /> },
          { path: '*', element: <NotFound /> },
        ],
      },
    ],
  },
]);
```

注意：`AuthProvider` 放在 `RequireAuth` 外层且只包住受保护区；登录页不需要 profile。`lazy` 的页面在 F4-F8 才创建——**本任务只创建 `pages/dashboard/index.tsx` 占位（下一步）与 login/403/404**，其余 lazy 页面在对应任务创建前会构建失败。为保持每步可构建：F3 先只保留 login/dashboard/403/404 路由，其余路由行在 F5-F8 各自任务里追加。

`src/pages/dashboard/index.tsx`（F4 会升级成欢迎卡片）：

```tsx
export default function Dashboard() {
  return <div>dashboard</div>;
}
```

- [ ] **Step 6: 验证构建**

Run: `cd admin/web && pnpm build`
Expected: 通过。（F3 阶段路由表只含 login/dashboard/403/404，`BasicLayout` 尚未创建 → 本步先不要引入 `BasicLayout`；改为 `element: <div />` 占位，F4 替换。）

- [ ] **Step 7: Commit**

```bash
git add admin/web/src
git commit -m "feat(admin-web): 鉴权上下文、路由守卫与登录页"
```

---

### Task F4: 布局（动态菜单 + 用户下拉）+ 首页 + 个人中心

**Files:**
- Create: `admin/web/src/layouts/BasicLayout.tsx`, `src/icons.ts`
- Create: `admin/web/src/pages/dashboard/index.tsx`（升级）, `src/pages/profile/index.tsx`
- Modify: `admin/web/src/router.tsx`（接入 BasicLayout）

- [ ] **Step 1: 写 `src/icons.ts`**

```tsx
import * as Icons from '@ant-design/icons';
import { AppstoreOutlined } from '@ant-design/icons';
import type { ComponentType } from 'react';

/** 菜单 icon 字符串 → antd 图标组件；不认识的名字回落默认图标。 */
export function menuIcon(name?: string): ComponentType {
  if (!name) return AppstoreOutlined;
  const Cmp = (Icons as unknown as Record<string, ComponentType>)[name];
  return Cmp ?? AppstoreOutlined;
}
```

- [ ] **Step 2: 写 `src/layouts/BasicLayout.tsx`**

```tsx
import { useMemo, useState } from 'react';
import { Avatar, Breadcrumb, Dropdown, Layout, Menu, theme } from 'antd';
import { LogoutOutlined, UserOutlined } from '@ant-design/icons';
import { Outlet, useLocation, useNavigate } from 'react-router';
import { menuIcon } from '../icons';
import { useAuth } from '../auth/AuthContext';
import type { MenuNode } from '../api/types';

const { Header, Sider, Content } = Layout;

interface MenuItem {
  key: string;
  icon: React.ReactNode;
  label: string;
  children?: MenuItem[];
}

function toItems(nodes: MenuNode[]): MenuItem[] {
  return nodes.map((n) => {
    const Icon = menuIcon(n.icon);
    return {
      key: n.path || String(n.id),
      icon: <Icon />,
      label: n.name,
      children: n.children?.length ? toItems(n.children) : undefined,
    };
  });
}

/** key → 菜单名，用于面包屑。 */
function flatten(nodes: MenuNode[], map: Record<string, string> = {}) {
  for (const n of nodes) {
    if (n.path) map[n.path] = n.name;
    if (n.children) flatten(n.children, map);
  }
  return map;
}

export default function BasicLayout() {
  const { user, menus, logout } = useAuth();
  const nav = useNavigate();
  const loc = useLocation();
  const [collapsed, setCollapsed] = useState(false);
  const { token } = theme.useToken();

  const items = useMemo(() => toItems(menus), [menus]);
  const names = useMemo(() => flatten(menus), [menus]);
  const crumbs = useMemo(() => {
    const parts = loc.pathname.split('/').filter(Boolean);
    const out: string[] = [];
    let acc = '';
    for (const p of parts) {
      acc += `/${p}`;
      out.push(names[acc] ?? p);
    }
    return out;
  }, [loc.pathname, names]);

  const userMenu = {
    items: [
      { key: 'profile', icon: <UserOutlined />, label: '个人中心' },
      { type: 'divider' as const },
      { key: 'logout', icon: <LogoutOutlined />, label: '退出登录' },
    ],
    onClick: async ({ key }: { key: string }) => {
      if (key === 'profile') nav('/profile');
      if (key === 'logout') {
        await logout();
        nav('/login', { replace: true });
      }
    },
  };

  return (
    <Layout style={{ minHeight: '100vh' }}>
      <Sider collapsible collapsed={collapsed} onCollapse={setCollapsed} theme="dark" width={220}>
        <div style={{ height: 48, margin: 12, color: '#fff', fontWeight: 600, textAlign: 'center', lineHeight: '48px' }}>
          {collapsed ? 'bee' : 'bee-rust 管理后台'}
        </div>
        <Menu
          theme="dark"
          mode="inline"
          items={items}
          selectedKeys={[loc.pathname]}
          defaultOpenKeys={['/' + (loc.pathname.split('/')[1] ?? '')]}
          onClick={({ key }) => { if (String(key).startsWith('/')) nav(String(key)); }}
        />
      </Sider>
      <Layout>
        <Header style={{ background: token.colorBgContainer, padding: '0 16px', display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
          <Breadcrumb items={crumbs.map((c) => ({ title: c }))} />
          <Dropdown menu={userMenu}>
            <span style={{ cursor: 'pointer' }}>
              <Avatar size="small" icon={<UserOutlined />} src={user?.avatar || undefined} />
              <span style={{ marginLeft: 8 }}>{user?.nickname || user?.username}</span>
            </span>
          </Dropdown>
        </Header>
        <Content style={{ margin: 16 }}>
          <Outlet />
        </Content>
      </Layout>
    </Layout>
  );
}
```

- [ ] **Step 3: 写 `src/pages/dashboard/index.tsx`（欢迎卡片）**

```tsx
import { Card, Descriptions, Tag } from 'antd';
import { useAuth } from '../../auth/AuthContext';

export default function Dashboard() {
  const { user, perms, menus } = useAuth();
  return (
    <Card title="欢迎回来">
      <Descriptions column={1} bordered size="small" style={{ maxWidth: 520 }}>
        <Descriptions.Item label="用户名">{user?.username}</Descriptions.Item>
        <Descriptions.Item label="昵称">{user?.nickname || '-'}</Descriptions.Item>
        <Descriptions.Item label="超级管理员">{user?.is_super ? <Tag color="gold">是</Tag> : '否'}</Descriptions.Item>
        <Descriptions.Item label="权限码数量">{perms.length}</Descriptions.Item>
        <Descriptions.Item label="可访问菜单数">{menus.length}</Descriptions.Item>
      </Descriptions>
    </Card>
  );
}
```

（「最近登录时间/IP」由后端 profile 返回时再加；当前契约里没有，等 M3 决定。**不要**前端先造字段。）

- [ ] **Step 4: 写 `src/pages/profile/index.tsx`**

```tsx
import { useState } from 'react';
import { App, Button, Card, Descriptions, Form, Input } from 'antd';
import { authApi } from '../../api/auth';
import { useAuth } from '../../auth/AuthContext';

export default function ProfilePage() {
  const { user, logout } = useAuth();
  const { message } = App.useApp();
  const [form] = Form.useForm();
  const [loading, setLoading] = useState(false);

  const onFinish = async (v: { old_password: string; new_password: string }) => {
    setLoading(true);
    try {
      await authApi.changePassword(v.old_password, v.new_password);
      message.success('密码已修改，请重新登录');
      form.resetFields();
      await logout();
      window.location.href = '/login';
    } finally {
      setLoading(false);
    }
  };

  return (
    <Card title="个人中心" style={{ maxWidth: 640 }}>
      <Descriptions column={1} size="small" style={{ marginBottom: 24 }}>
        <Descriptions.Item label="用户名">{user?.username}</Descriptions.Item>
        <Descriptions.Item label="昵称">{user?.nickname || '-'}</Descriptions.Item>
      </Descriptions>
      <Form form={form} layout="vertical" onFinish={onFinish} style={{ maxWidth: 360 }}>
        <Form.Item name="old_password" label="原密码" rules={[{ required: true, message: '请输入原密码' }]}>
          <Input.Password autoComplete="current-password" />
        </Form.Item>
        <Form.Item
          name="new_password"
          label="新密码"
          rules={[{ required: true, min: 6, message: '新密码至少 6 位' }]}
        >
          <Input.Password autoComplete="new-password" />
        </Form.Item>
        <Form.Item
          name="confirm"
          label="确认新密码"
          dependencies={['new_password']}
          rules={[
            { required: true, message: '请再次输入新密码' },
            ({ getFieldValue }) => ({
              validator: (_, value) =>
                !value || getFieldValue('new_password') === value
                  ? Promise.resolve()
                  : Promise.reject(new Error('两次输入不一致')),
            }),
          ]}
        >
          <Input.Password autoComplete="new-password" />
        </Form.Item>
        <Button type="primary" htmlType="submit" loading={loading}>保存</Button>
      </Form>
    </Card>
  );
}
```

- [ ] **Step 5: 接入 `src/router.tsx`**

把 F3 里的 `element: <div />` 占位换成 `element: <BasicLayout />`；`dashboard`、`profile` 路由改为指向真实页面。

- [ ] **Step 6: 验证构建 + 起 dev server 冒烟**

Run: `cd admin/web && pnpm build`
Expected: 通过。

Run: `cd admin/web && (pnpm dev &) && sleep 3 && curl -s localhost:5173 | head -5 && kill %1`
Expected: 返回包含 `<div id="root">` 的 HTML（说明 dev server 起来了）。

- [ ] **Step 7: Commit**

```bash
git add admin/web/src
git commit -m "feat(admin-web): 布局（动态菜单/面包屑/用户下拉）、首页与个人中心"
```

---

### Task F5: 管理员管理页

**Files:**
- Create: `admin/web/src/pages/system/admin/index.tsx`

- [ ] **Step 1: 写页面（表格 + 搜索 + 新增/编辑弹窗 + 分配角色 + 重置密码 + 状态开关 + 删除）**

```tsx
import { useCallback, useEffect, useState } from 'react';
import {
  App, Button, Form, Input, Modal, Popconfirm, Select, Space, Switch, Table, Tag,
} from 'antd';
import { PlusOutlined, ReloadOutlined } from '@ant-design/icons';
import type { ColumnsType } from 'antd/es/table';
import { adminApi, type AdminForm, type AdminQuery } from '../../../api/admin';
import { roleApi } from '../../../api/role';
import type { Admin, Role } from '../../../api/types';
import { deptApi } from '../../../api/dept';
import type { Dept } from '../../../api/types';
import Auth from '../../../auth/Auth';
import { useAuth } from '../../../auth/AuthContext';

/** 部门树 → antd TreeSelect 数据。 */
function toTreeData(nodes: Dept[]): { value: number; title: string; children?: unknown[] }[] {
  return nodes.map((d) => ({
    value: d.id,
    title: d.name,
    children: d.children?.length ? toTreeData(d.children) : undefined,
  }));
}

export default function AdminPage() {
  const { message } = App.useApp();
  const { user: me } = useAuth();
  const [form] = Form.useForm<AdminForm>();
  const [query, setQuery] = useState<AdminQuery>({ page: 1, size: 10 });
  const [rows, setRows] = useState<Admin[]>([]);
  const [total, setTotal] = useState(0);
  const [loading, setLoading] = useState(false);
  const [roles, setRoles] = useState<Role[]>([]);
  const [depts, setDepts] = useState<Dept[]>([]);
  const [modalOpen, setModalOpen] = useState(false);
  const [editing, setEditing] = useState<Admin | null>(null);

  const load = useCallback(async (q: AdminQuery) => {
    setLoading(true);
    try {
      const res = await adminApi.list(q);
      setRows(res.list);
      setTotal(res.total);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { void load(query); }, [query, load]);
  useEffect(() => {
    void roleApi.list({ page: 1, size: 200 }).then((r) => setRoles(r.list));
    void deptApi.tree().then(setDepts);
  }, []);

  const openCreate = () => {
    setEditing(null);
    form.resetFields();
    form.setFieldsValue({ status: 1, sex: 0, dept_id: 0, role_ids: [] } as unknown as AdminForm);
    setModalOpen(true);
  };

  const openEdit = async (row: Admin) => {
    const detail = await adminApi.get(row.id);
    setEditing(detail);
    form.setFieldsValue({
      nickname: detail.nickname, email: detail.email, phone: detail.phone, sex: detail.sex,
      dept_id: detail.dept_id, status: detail.status, remark: detail.remark,
      role_ids: detail.role_ids ?? [],
    } as unknown as AdminForm);
    setModalOpen(true);
  };

  const submit = async () => {
    const v = await form.validateFields();
    if (editing) {
      await adminApi.update(editing.id, v);
      message.success('已保存');
    } else {
      await adminApi.create(v);
      message.success('已创建');
    }
    setModalOpen(false);
    void load(query);
  };

  const columns: ColumnsType<Admin> = [
    { title: 'ID', dataIndex: 'id', width: 70 },
    { title: '用户名', dataIndex: 'username' },
    { title: '昵称', dataIndex: 'nickname' },
    { title: '部门', dataIndex: 'dept_name', render: (v) => v || '-' },
    {
      title: '角色', dataIndex: 'role_names',
      render: (v: string[], row) =>
        row.is_super ? <Tag color="gold">超级管理员</Tag> : (v ?? []).map((n) => <Tag key={n}>{n}</Tag>),
    },
    {
      title: '状态', dataIndex: 'status', width: 90,
      render: (v: number, row) => (
        <Auth code="system:admin:edit">
          <Switch
            checked={v === 1}
            disabled={row.id === me?.id || row.is_super}
            onChange={async (checked) => {
              await adminApi.setStatus(row.id, checked ? 1 : 0);
              message.success('已更新');
              void load(query);
            }}
          />
        </Auth>
      ),
    },
    { title: '创建时间', dataIndex: 'created_at', width: 170 },
    {
      title: '操作', width: 240, fixed: 'right',
      render: (_, row) => (
        <Space>
          <Auth code="system:admin:edit">
            <Button size="small" type="link" onClick={() => void openEdit(row)}>编辑</Button>
          </Auth>
          <Auth code="system:admin:resetPwd">
            <Button
              size="small" type="link"
              disabled={row.is_super && row.id !== me?.id}
              onClick={() => {
                let pwd = '';
                Modal.confirm({
                  title: `重置 ${row.username} 的密码`,
                  content: (
                    <Input.Password placeholder="新密码（至少 6 位）" onChange={(e) => { pwd = e.target.value; }} />
                  ),
                  onOk: async () => {
                    if (pwd.length < 6) {
                      message.error('密码至少 6 位');
                      return Promise.reject(new Error('too short'));
                    }
                    await adminApi.resetPassword(row.id, pwd);
                    message.success('已重置');
                  },
                });
              }}
            >
              重置密码
            </Button>
          </Auth>
          <Auth code="system:admin:remove">
            <Popconfirm
              title="确认删除该管理员？"
              onConfirm={async () => { await adminApi.remove(row.id); message.success('已删除'); void load(query); }}
            >
              <Button
                size="small" type="link" danger
                disabled={row.id === me?.id || row.is_super}
              >
                删除
              </Button>
            </Popconfirm>
          </Auth>
        </Space>
      ),
    },
  ];

  return (
    <>
      <Space style={{ marginBottom: 16 }} wrap>
        <Input.Search
          placeholder="用户名" allowClear style={{ width: 200 }}
          onSearch={(v) => setQuery((q) => ({ ...q, username: v || undefined, page: 1 }))}
        />
        <Select
          placeholder="状态" allowClear style={{ width: 120 }}
          options={[{ value: 1, label: '启用' }, { value: 0, label: '禁用' }]}
          onChange={(v) => setQuery((q) => ({ ...q, status: v, page: 1 }))}
        />
        <Button icon={<ReloadOutlined />} onClick={() => void load(query)}>刷新</Button>
        <Auth code="system:admin:add">
          <Button type="primary" icon={<PlusOutlined />} onClick={openCreate}>新增</Button>
        </Auth>
      </Space>

      <Table<Admin>
        rowKey="id"
        size="small"
        loading={loading}
        columns={columns}
        dataSource={rows}
        scroll={{ x: 1100 }}
        pagination={{
          current: query.page, pageSize: query.size, total, showSizeChanger: true,
          onChange: (page, size) => setQuery((q) => ({ ...q, page, size })),
        }}
      />

      <Modal
        title={editing ? `编辑管理员：${editing.username}` : '新增管理员'}
        open={modalOpen}
        onCancel={() => setModalOpen(false)}
        onOk={() => void submit()}
        destroyOnClose
        width={560}
      >
        <Form form={form} labelCol={{ span: 5 }} wrapperCol={{ span: 18 }}>
          {!editing && (
            <>
              <Form.Item name="username" label="用户名" rules={[{ required: true, min: 3, message: '至少 3 位' }]}>
                <Input autoComplete="off" />
              </Form.Item>
              <Form.Item name="password" label="初始密码" rules={[{ required: true, min: 6, message: '至少 6 位' }]}>
                <Input.Password autoComplete="new-password" />
              </Form.Item>
            </>
          )}
          <Form.Item name="nickname" label="昵称" rules={[{ required: true, message: '请输入昵称' }]}>
            <Input />
          </Form.Item>
          <Form.Item name="dept_id" label="部门">
            <Select
              allowClear placeholder="选择部门"
              options={[{ value: 0, label: '（无）' }, ...toTreeData(depts).map((d) => ({ value: d.value, label: d.title }))]}
            />
          </Form.Item>
          <Form.Item name="role_ids" label="角色">
            <Select
              mode="multiple" allowClear placeholder="选择角色"
              options={roles.map((r) => ({ value: r.id, label: r.name }))}
            />
          </Form.Item>
          <Form.Item name="email" label="邮箱"><Input /></Form.Item>
          <Form.Item name="phone" label="手机号"><Input /></Form.Item>
          <Form.Item name="sex" label="性别">
            <Select options={[{ value: 0, label: '未知' }, { value: 1, label: '男' }, { value: 2, label: '女' }]} />
          </Form.Item>
          <Form.Item name="status" label="状态">
            <Select options={[{ value: 1, label: '启用' }, { value: 0, label: '禁用' }]} />
          </Form.Item>
          <Form.Item name="remark" label="备注"><Input.TextArea rows={2} /></Form.Item>
        </Form>
      </Modal>
    </>
  );
}
```

部门选择这里用扁平的 `Select`（`toTreeData` 拍平）——若你用 `TreeSelect` 需要 `treeData` + `fieldNames`，两者都可，实现时二选一并保证 `dept_id` 提交的是 number。

- [ ] **Step 2: 在 `src/router.tsx` 追加路由**

`{ path: 'system/admin', element: <L><AdminPage /></L> }`

- [ ] **Step 3: 验证构建**

Run: `cd admin/web && pnpm build`
Expected: 通过。

- [ ] **Step 4: Commit**

```bash
git add admin/web/src
git commit -m "feat(admin-web): 管理员管理页（列表/搜索/增删改/状态/重置密码/角色分配）"
```

---

### Task F6: 角色管理页（权限树 + 数据权限）

**Files:**
- Create: `admin/web/src/pages/system/role/index.tsx`

- [ ] **Step 1: 写页面**

要点（实现时必须全部满足）：

- 列表列：ID / 角色名 / 角色标识 / 排序 / 数据范围（`DATA_SCOPE_LABELS`）/ 状态 / 创建时间 / 操作
- 操作：编辑、权限（抽屉）、删除
- 编辑弹窗字段：name（必填）、code（必填，编辑时禁用）、sort、data_scope（Select，选项来自 `DATA_SCOPE_LABELS`）、status、remark
- 权限抽屉：`menuApi.tree()` 取全量菜单树，`roleApi.menus(id)` 取已选 id，antd `Tree`（`checkable`，`checkedKeys`，`onCheck`）保存 `roleApi.setMenus`
- 数据范围 = 自定义（5）时：抽屉里追加「数据权限」区块，`deptApi.tree()` + `roleApi.depts(id)`，保存时 `roleApi.setDepts`
- 树勾选细节：父节点半选时 antd 只回传叶子，保存前用 `tree.checkStrictly = false` 的默认行为即可（后端按 id 集合存）

结构与 F5 一致（Search 表单 + Table + Modal），关键代码：

```tsx
const [treeData, setTreeData] = useState<Menu[]>([]);
const [permOpen, setPermOpen] = useState(false);
const [permRole, setPermRole] = useState<Role | null>(null);
const [checked, setChecked] = useState<number[]>([]);
const [deptChecked, setDeptChecked] = useState<number[]>([]);

const openPerm = async (row: Role) => {
  setPermRole(row);
  const [tree, ids, deptIds] = await Promise.all([
    menuApi.tree(), roleApi.menus(row.id), roleApi.depts(row.id),
  ]);
  setTreeData(tree);
  setChecked(ids);
  setDeptChecked(deptIds);
  setPermOpen(true);
};

const savePerm = async () => {
  if (!permRole) return;
  await roleApi.setMenus(permRole.id, checked);
  if (permRole.data_scope === 5) await roleApi.setDepts(permRole.id, deptChecked);
  message.success('权限已保存');
  setPermOpen(false);
};

// 菜单树 → antd Tree
const toTree = (nodes: Menu[]): DataNode[] =>
  nodes.map((n) => ({ key: n.id, title: `${n.name}${n.type === 'F' ? '（按钮）' : ''}`, children: n.children ? toTree(n.children) : undefined }));
```

- [ ] **Step 2: 路由追加** `{ path: 'system/role', element: <L><RolePage /></L> }`

- [ ] **Step 3: 验证构建** Run: `cd admin/web && pnpm build` → 通过

- [ ] **Step 4: Commit**

```bash
git add admin/web/src
git commit -m "feat(admin-web): 角色管理页（CRUD + 菜单权限树 + 数据范围/自定义部门）"
```

---

### Task F7: 菜单管理页 + 部门管理页

**Files:**
- Create: `admin/web/src/pages/system/menu/index.tsx`, `src/pages/system/dept/index.tsx`

- [ ] **Step 1: 菜单页**

要点：

- `Table` 用 `menuApi.tree()` 的树数据：antd 5 直接给 `dataSource` 带 `children` 的树 + 不需要额外配置（`childrenColumnName` 默认 `children`）
- 列：名称 / 类型（M 目录、C 菜单、F 按钮，Tag 区分）/ 权限码 / 路径 / 排序 / 状态 / 操作（新增子项、编辑、删除）
- 编辑弹窗字段：parent_id（TreeSelect，来自当前树，可空=顶级）、name、type（Radio：M/C/F）、perm（按钮/菜单填）、path、component、icon、sort、visible、status
- 类型切换时按需禁用字段：M 显示 path、C 显示 path+component+perm、F 只显示 perm

- [ ] **Step 2: 部门页**

要点：

- `Table` 树数据来自 `deptApi.tree()`，列：名称 / 负责人 / 电话 / 排序 / 状态 / 操作（新增子部门、编辑、删除）
- 编辑弹窗字段：parent_id（TreeSelect）、name、sort、leader、phone、status

- [ ] **Step 3: 路由追加** `system/menu`、`system/dept` 两行

- [ ] **Step 4: 验证构建** Run: `cd admin/web && pnpm build` → 通过

- [ ] **Step 5: Commit**

```bash
git add admin/web/src
git commit -m "feat(admin-web): 菜单管理页与部门管理页（树形表格 + 增删改）"
```

---

### Task F8: 登录记录页 + 全量收尾

**Files:**
- Create: `admin/web/src/pages/system/loginlog/index.tsx`
- Create: `admin/web/README.md`

- [ ] **Step 1: 登录记录页**

要点：

- 搜索：用户名 Input、状态 Select（成功/失败）、时间范围 `DatePicker.RangePicker`（提交时 `start`/`end` 转 `YYYY-MM-DD HH:mm:ss`）
- 列：ID / 用户名 / IP / 状态（Tag 红绿）/ 说明 / User-Agent（超长省略）/ 时间
- 操作：清空（`Popconfirm`，带当前筛选条件 → `loginLogApi.clear(query)`），需 `system:loginlog:remove` 权限包裹

- [ ] **Step 2: 路由追加** `{ path: 'system/login-log', element: <L><LoginLogPage /></L> }`

- [ ] **Step 3: 全量构建验证**

Run: `cd admin/web && pnpm build`
Expected: 通过，产出 `dist/`。

- [ ] **Step 4: 写 `admin/web/README.md`**

内容：开发（`pnpm install && pnpm dev`，代理 `/api` → `127.0.0.1:8080`）、构建（`pnpm build` 产出 `dist/`）、部署（nginx root 指向 dist，`try_files $uri /index.html`）、后端见仓库根 README。

- [ ] **Step 5: Commit**

```bash
git add admin/web
git commit -m "feat(admin-web): 登录记录页、README 与全量构建验证"
```

---

## 明知取舍

- **端到端点击验证推迟到 M6**：后端起来前只能验证「能构建、能起 dev server」；页面与真实接口的联调不在本计划内假装完成。
- **不做前端单测**：交互逻辑薄（表格/表单/权限显示），成本高于收益；权限显示逻辑靠 M6 手验。
- **不做主题/暗色/国际化切换**：固定中文 + antd 默认主题。
- **首页统计卡片不做**：契约里没有统计接口，不造字段。

## Self-Review 记录

- 覆盖设计文档 §6 全部页面（登录/首页/管理员/角色/菜单/部门/登录记录/个人中心 + 403/404）。
- 契约与设计文档 §5.3 一致；`profile`/`menus` 的 JSON 形状在此钉死，M3 必须按此实现。
- 路由与懒加载在 F3 建立，F5-F8 逐步追加（每步可构建）。
- 已知偏差：React 18.3 代替设计文档里的 React 19（antd 5 零补丁组合）；首页去掉「最近登录时间/IP」（契约无此字段，M3 若加上再补）。
