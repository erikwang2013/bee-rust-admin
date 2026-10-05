# BRD 后端镜像（bee_admin）
#
# 构建（在仓库根目录执行）：
#   docker build -t brd-backend .
# 运行（必须挂载配置文件，否则启动时读不到 app.conf 会直接退出）：
#   docker run -d --name brd -p 8080:8080 \
#     -v "$PWD/admin/conf/app.conf:/app/conf/app.conf:ro" \
#     -e BEE_ADMIN_DB_DSN='mysql://user:pass@host:3306/bee_admin' \
#     brd-backend
#
# 说明：
#   - 配置文件里的 [db] dsn 会被 BEE_ADMIN_DB_DSN 覆盖（容器里指向 MySQL 服务名更自然）
#   - [jwt] secret 与 [seed] initial_admin_password 仍从挂载的 app.conf 读取

# ── 构建阶段 ───────────────────────────────────────────────
# 注意：构建阶段与运行阶段必须是同一个 Debian 版本（这里是 trixie / Debian 13），
# 否则容器内 glibc 版本低于编译时所用版本，二进制一启动就报 GLIBC_x.xx not found。
FROM rust:1.98.1-slim AS builder
WORKDIR /src
COPY Cargo.toml Cargo.lock rustfmt.toml ./
COPY crates ./crates
COPY admin/Cargo.toml ./admin/Cargo.toml
COPY admin/src ./admin/src
# workspace 的 members 里包含示例 crate，cargo 解析清单时需要它（不会编译它）
COPY examples ./examples
# 说明：这里刻意不用 BuildKit 的 RUN --mount=type=cache（CI 上会更快，但旧版
# legacy builder 不支持该语法）。需要更快重复构建时，把本行换成：
#   RUN --mount=type=cache,target=/usr/local/cargo/registry \
#       --mount=type=cache,target=/src/target \
#       cargo build --release -p bee_admin && cp /src/target/release/bee_admin /src/bee_admin.release
RUN cargo build --release -p bee_admin \
    && cp /src/target/release/bee_admin /src/bee_admin.release

# ── 运行阶段 ───────────────────────────────────────────────
# 与构建阶段同为 Debian 13（trixie）；有 trixie-slim 时可换成更小的镜像
FROM debian:13
# libgomp1：release 二进制链接了 OpenMP 运行时（libgomp.so.1），slim 基础镜像不自带
RUN apt-get update \
    && apt-get install -y --no-install-recommends libgomp1 ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -r -u 10001 -m app

WORKDIR /app
COPY --from=builder /src/bee_admin.release /app/bee_admin
COPY admin/conf/app.conf.example /app/conf/app.conf.example

ENV BEE_ADMIN_CONF=/app/conf/app.conf
EXPOSE 8080
USER app
CMD ["/app/bee_admin"]
