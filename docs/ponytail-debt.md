# ponytail 有意搁置台账

这是全仓库「有意搁置」的汇总：每条都是源码里带 `ponytail:` 注释的一处**有意**简化——已知天花板 + 升级路径，不是 bug 列表，不用顺手去还。
用法：动到某个模块前先扫一眼对应分组；一旦触发了「升级触发条件」，就按该行的路径升级，并同步更新/删掉源码里的注释（注释本身才是唯一真相，本台账只是视图）。

## admin 应用（`admin/src/`）

| 位置 | 有意搁置什么（天花板） | 升级触发条件 / 路径 |
|---|---|---|
| [`admin/src/api/admin.rs:192`](../admin/src/api/admin.rs#L192) | 管理员列表每行一次关联查询（页 10 行 = 11 次查询） | 行数大改批量查 |
| [`admin/src/api/auth.rs:174`](../admin/src/api/auth.rs#L174) | `/captcha/new` 限流阈值 60 次/分钟是拍的：正常用户一分钟开不了 60 次登录页，刷子被压到 60 条/分钟（按 TTL 300s 算单 IP 稳态 ≤ 300 条） | 要调就调这两个数（`threshold` / `window_secs`） |
| [`admin/src/api/auth.rs:214`](../admin/src/api/auth.rs#L214) | 验证码全局限流的额度只在单进程内计数（多实例各算各的，总限量按实例数翻倍） | 多实例部署时挪到网关或换跨进程存储 |
| [`admin/src/api/auth.rs:237`](../admin/src/api/auth.rs#L237) | 验证码「消费」的互斥锁只在单进程内有效 | 多实例部署要换插件的 Redis 存储，那才是真正跨进程的消费 |
| [`admin/src/api/login_log.rs:78`](../admin/src/api/login_log.rs#L78) | 清空日志先全量取 id 再分批删（ORM 无按条件批量删） | 日志量极大时改为 ORM 批量删 |
| [`admin/src/datascope.rs:207`](../admin/src/datascope.rs#L207) | 每行一次 `role_perms`（2 条查询）；角色是配置量级（几十个），不分批 | 注释未给触发条件 |
| [`admin/src/jobs.rs:10`](../admin/src/jobs.rs#L10) | 调度假定单实例：同进程内靠 `last_run_at` 抢占，多实例下同一任务会重复跑 | 多实例部署前换成 DB 行锁 |

## crates 框架（`crates/`）

| 位置 | 有意搁置什么（天花板） | 升级触发条件 / 路径 |
|---|---|---|
| [`crates/bee_graph/src/nebulagraph.rs:80`](../crates/bee_graph/src/nebulagraph.rs#L80) | nGQL `UPDATE VERTEX` 需要 tag 名而 trait 不携带：用通配 tag `*` 保持调用点简单 | 后端拒绝 `*` 时给 driver 加 tag 字段并在这里用它 |
| [`crates/bee_logs/src/lib.rs:11`](../crates/bee_logs/src/lib.rs#L11) | `Logger` 没有异步开关：原 `async_mode` 字段与 `async_()` setter 从未被 `init()` 读取，是假开关（已删）；异步与否由 `Output` 变体决定（`File`/`MultiFile` 走 non_blocking worker，`Stdout` 同步） | `Stdout` 也被阻塞拖慢时再加 |
| [`crates/bee_orm/src/lib.rs:44`](../crates/bee_orm/src/lib.rs#L44) | `MAX_BIND_PARAMS = 999`：所有后端统一按 sqlite 的经典上限分块（pg/mysql 允许 65535），换「一个常量、统一 SQL」 | 注释未给触发条件 |
| [`crates/bee_orm/src/pool/mysql.rs:41`](../crates/bee_orm/src/pool/mysql.rs#L41) | 取连接超时写死 30s（mysql_async 0.34.2 没有 acquire timeout） | 调用方需要调时改成可配置 |
| [`crates/bee_orm/src/pool/mysql.rs:219`](../crates/bee_orm/src/pool/mysql.rs#L219) | `bytes` 列按 UTF-8 文本解码；日期时间列在不带 `chrono` 特性时返回 null（带则按列类型分发） | 注释未给触发条件（`chrono` 是既有特性开关，不是升级路径） |
| [`crates/bee_orm/src/pool/postgres.rs:72`](../crates/bee_orm/src/pool/postgres.rs#L72) + [`:116`](../crates/bee_orm/src/pool/postgres.rs#L116) | 连接池超时写死（wait 30s / create 10s / recycle 无）；`connect` 与 `connect_tls_with` 各写一份，同一处天花板 | 调用方需要其他值时做成 `PoolConfig` 参数 |
| [`crates/bee_orm/src/pool/postgres.rs:262`](../crates/bee_orm/src/pool/postgres.rs#L262) | `pg_sql` 盲替换所有 `?` → `$n`：前提是 `QuerySet` 只发 `?`、`filter()` 原始串是可信常量；手写 SQL 字符串字面量里的 `?` 也会被改写 | 注释未给触发条件 |
| [`crates/bee_orm/src/pool/postgres.rs:288`](../crates/bee_orm/src/pool/postgres.rs#L288) | 只解码常见列类型；数组列与 timestamp-without-time-zone 系列返回 null（日期/数值随 `chrono` / `rust_decimal` 特性解码，关闭时落 `_` 分支 null） | 注释未给触发条件 |
| [`crates/bee_tsdb/src/iotdb.rs:55`](../crates/bee_tsdb/src/iotdb.rs#L55) | `query_range` 无服务端过滤下推，tag 过滤在客户端解析后应用 | 吞吐成为问题时加上 |
| [`crates/bee_tsdb/src/iotdb.rs:171`](../crates/bee_tsdb/src/iotdb.rs#L171) | IoTDB REST 不引用字符串值：含逗号的字符串会把行拆成多余片段（末列吸收多余部分），多列逗号仍会错位 | 真出问题时换成不含逗号的服务端格式 |
| [`crates/bee_tsdb/src/questdb.rs:41`](../crates/bee_tsdb/src/questdb.rs#L41) | `write_batch` 多行 VALUES 要求整批列布局一致，混合 schema 会错位 | 注释未给触发条件 |
| [`crates/bee_tsdb/src/questdb.rs:79`](../crates/bee_tsdb/src/questdb.rs#L79) | QuestDB 无原生 CQ：`create_continuous_query` 是 no-op | 需要时再实现 |

注：`postgres.rs:72` / `:116` 为同一处天花板，合并为一行。

---

共 20 条注释（本台账 19 行，`postgres.rs` 池超时 2 处合并 1 行）：admin 应用 7 条、crates 框架 13 条。
