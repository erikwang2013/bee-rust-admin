# BRA 前端镜像（静态资源 + nginx 反代）
#
# 构建（在 admin/web 目录执行）：
#   docker build -t bra-web .
# 运行：
#   docker run -d --name bra-web -p 8081:80 \
#     -e BACKEND_UPSTREAM=bee-admin:8080 bra-web
#
# 说明：nginx 官方镜像的 entrypoint 会对 /etc/nginx/templates/*.template
#       做 envsubst 后输出到 /etc/nginx/conf.d/，所以上游地址用环境变量注入。

# ── 构建阶段 ───────────────────────────────────────────────
FROM node:22-alpine AS builder
WORKDIR /app
RUN corepack enable
COPY package.json pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile
COPY . .
RUN pnpm build

# ── 运行阶段 ───────────────────────────────────────────────
FROM nginx:alpine
COPY --from=builder /app/dist /usr/share/nginx/html
COPY nginx.conf.template /etc/nginx/templates/default.conf.template

# 后端上游（compose 里的服务名 + 端口）；可用环境变量覆盖
ENV BACKEND_UPSTREAM=bee-admin:8080
EXPOSE 80
