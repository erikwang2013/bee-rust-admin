# 发版

发版的固定动作全在 [`scripts/release.sh`](../scripts/release.sh) 里，这篇讲为什么是这些动作、
产物口径是什么、发完怎么验。**升级现有部署**前的检查清单在[发布 / 升级检查清单](release-checklist.md)。

## 一条命令

```bash
bash scripts/release.sh v2.9.0 --notes-file /tmp/notes-v2.9.0.md
bash scripts/release.sh v2.9.0 --notes-file /tmp/notes-v2.9.0.md --dry-run   # 真构建真打包，不发
bash scripts/release.sh v2.9.0 --notes-file /tmp/notes-v2.9.0.md --skip-images  # 本机没 docker，只发二进制
```

前置（脚本自己查，一个不满足就退出、什么都不发）：

- 在 `main` 分支上、**工作树干净** —— 构建出来的东西必须与 tag 指向的那个提交完全一致
- HEAD 已推送（`git rev-parse HEAD` == `git rev-parse origin/main`）
- tag `vX.Y.Z` 本地与远端都没用过（发过的版本不重发）
- `gh` 已登录

`--dry-run` 会真构建、真打包、真自检，然后在打印出将要执行的 `gh release create` 命令后停下 ——
不推送、不建 release、不动远端。机器上没到发版条件时（比如不在 `main`），干跑把不满足的项列成
警告继续跑完，好让你先把产物验一遍。

改过 `install.sh` 之后，另跑一次 `bash scripts/smoke-install.sh`（在 `/tmp` 装一套 scratch 实例、
再 `--keep-conf` 升级一次、断言、清理；全程不碰生产的 8080 与 `bee_admin` 库）。

## 产物口径

| 资产名 | 是什么 |
|---|---|
| `bee_admin` | 后端 ELF，**`strip` 过** |
| `bee-admin-web-dist.tar.gz` | 前端 dist，tar 根下是 `./index.html` 与 `./assets/` |
| `bra-images-vX.Y.Z.tar.gz` | 后端 + 前端镜像（`bra-backend:$TAG`/`bra-web:$TAG` 的 `docker save`；`docker load -i` 即导入） |
| `bra-compose-vX.Y.Z.tar.gz` | 离线 compose 包：`docker-compose.yml` + 预填 `.env`（MySQL 口令 / `BRA_VERSION=$TAG`）+ `app.conf.example` + 一页说明 |

- **前两个给二进制安装**（`install.sh` 的 download 模式按名拉），**后两个给 Docker / compose
  部署**（镜像只在本机构建、离线分发，随 release 附带，不推任何镜像仓库）。包里的 README
  写清了解包 → `docker load` → 改两处口令 → `docker compose up -d`（不加 `--build`）这条路。
- **`--skip-images`**：本机没有 docker daemon 时用它，只发前两个产物（镜像与 compose 包
  那一节整个跳过，`gh` 资产列表也只剩两个）。
- **一定要 strip**：不 strip 的二进制 22MB，strip 掉符号表后 ~16.6MB。这是发布口径的一部分，不是可选优化。
- **资产名固定**：`install.sh` 的 download 模式按上面这两个名字去
  `releases/download/<tag>/` 拉；改名 = 所有一键安装直接挂掉。
- **tar 结构固定**：`tar czf … -C admin/web/dist .`，根下直接是 `./index.html` —— `install.sh`
  就是靠它认产物的（解压后找不到 `index.html` 会直接失败）。
- **Release 标题只写 `vX.Y.Z`**，描述全部写在 notes 文件里。

## 为什么 CI 不构建产物

`.github/workflows/test.yml` 只跑测试，不发布、不上传任何产物（workflow 里写了「镜像仍在本机构建」）。
镜像同理：只在本机构建、离线分发 —— 随 Release 附带的是 `docker save` 出来的包，不是推给某个镜像仓库。
预编译产物有一条硬门槛：**需要 glibc ≥ 2.38**（Debian 13 / Ubuntu 24.04+）。
在别的镜像里构建等于换一套运行时基线，先得把这条门槛重新评估 —— 所以产物只从本机发。

## 发完怎么验

1. 各资产能下（前两个就是 `install.sh --version <tag>` 拉的那两个）：

   ```
   https://github.com/erikwang2013/bee-rust-admin/releases/download/v2.9.0/bee_admin
   https://github.com/erikwang2013/bee-rust-admin/releases/download/v2.9.0/bee-admin-web-dist.tar.gz
   https://github.com/erikwang2013/bee-rust-admin/releases/download/v2.9.0/bra-images-v2.9.0.tar.gz
   https://github.com/erikwang2013/bee-rust-admin/releases/download/v2.9.0/bra-compose-v2.9.0.tar.gz
   ```

2. 装一套试试（默认 download 模式，会自己跑可运行性检查与健康检查）：

   ```bash
   bash install.sh --version v2.9.0 --prefix /tmp/bra-try
   ```

## 回滚

- **用户侧**：`mv $PREFIX/bin/bee_admin.prev $PREFIX/bin/bee_admin && systemctl restart bee-admin`
  —— `install.sh` 每次换二进制都把上一版留成 `bee_admin.prev`，只回滚二进制，库结构与数据不动。
- **Release 侧**：发错了就 `gh release delete vX.Y.Z`、删掉 tag，修完按同一个版本号重发
  （已经装过这个版本的用户不会自动退回去）。
