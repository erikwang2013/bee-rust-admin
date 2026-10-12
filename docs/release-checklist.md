# 发布 / 升级检查清单

每次部署 / 升级前过一遍，逐条打勾；有对不上的，先修完再动手。

## 升级前

- [ ] `admin/conf/app.conf` 与示例对一遍（`admin/conf/app.conf.example`；Docker 部署的库口令另在
      `.env.example`），这 5 个键一个不能少：`[app] encrypt_key`（**必填**，缺了 / 留占位符拒绝启动）、
      `hashids_salt`、`hashids_min_len`、`snowflake_worker`、`snowflake_dc`
      （后两个**多实例部署时每个进程必须不同**，否则撞号）
- [ ] 升级时 `encrypt_key` / `hashids_salt` 保持**原值**——`encrypt_key` 换了 = 库里已加密的
      email / phone 全部解不开；`hashids_salt` 换了 = 已发出去的短串 id 全部作废
- [ ] **前后端同版本换**：自 v2.5 起头像走分片上传（`POST /api/v1/auth/avatar` 已删），
      只升后端 → 头像 404
- [ ] 确实要先换后端、前端稍后再重建的过渡期：把 `[auth] captcha` 写成 `false`，
      否则老前端的登录会被判 400（验证码错误）
- [ ] 告知用户：升级窗口内需要**重新登录**（token 与版本相关，旧票不认）

## 升级后

- [ ] 抽查**登录** + **个人资料**保存一次：能正常读写 = `encrypt_key` 解密正常
- [ ] 用**旧版 `install.sh --keep-conf`** 装/升过的部署：核对 conf 里 `[db] dsn` 的口令与库里账号
      是否一致（旧脚本会 `ALTER USER` 把库口令改飞，conf 与库对不上就连不上库；v2.8.0 起已修）
- [ ] 确认解密正常、且不需要回滚之后，明文备份表 `admin_email_phone_backup` 按
      `docs/usage.md` 的运维小节（本批新增）清理

## 回滚（如需）

- [ ] `mv $PREFIX/bin/bee_admin.prev $PREFIX/bin/bee_admin && systemctl restart bee-admin`
      —— 上次安装 / 升级留下的旧二进制就在 `bee_admin.prev`，一条 `mv` 加重启回到旧版
      （只回滚二进制，数据库结构与数据不动）
