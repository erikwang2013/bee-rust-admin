# bee-rust 管理后台前端

Vite + React 18 + TypeScript + antd 5 单页应用。后端契约见 `docs/superpowers/specs/2026-10-05-bee-rust-admin-design.md` §5.3。

## 开发

```bash
pnpm install
pnpm dev          # http://localhost:5173，/api 代理到 http://127.0.0.1:8080
```

## 构建

```bash
pnpm build        # tsc -b + vite build，产出 dist/
pnpm preview      # 本地预览 dist/
```

## 部署

nginx 把 root 指向 `dist/`，SPA 路由回落到 index.html：

```nginx
location / {
  root /path/to/admin/web/dist;
  try_files $uri /index.html;
}
location /api/ {
  proxy_pass http://127.0.0.1:8080;
}
```

## 鉴权

JWT 存 localStorage（key `bee_admin_token`），axios 拦截器注入 `Authorization: Bearer`，HTTP 401 回登录页。
菜单与按钮权限由 `/api/v1/auth/menus`、`/api/v1/auth/profile` 驱动，前端 `<Auth code="...">` 只做显示控制，后端仍逐接口鉴权。

## 后端

见仓库根目录 README。
