# BRA · Bee Rust Admin

基于 bee-rust 框架的 RBAC 管理后台 —— Rust 服务端 + React 前端，建库即用。

`JWT 登录` · `管理员 / 角色 / 菜单 / 部门 / 字典 / 定时任务 / 通知公告 / 登录记录 / 操作日志` · `菜单 + 按钮级权限` · `部门数据权限` · `中英双语` · `Docker 一键部署`

---

## 项目宠物

![阿守（Keeper）· BRA 项目宠物：站在六边形巢门平台上、爪挂三把钥匙的值守蜂](docs/pet/keeper-onduty.svg)

**阿守（Keeper）** —— 蜂巢的值守蜂。

框架宠物 [Rusty](docs/rusty.svg) 是造蜂巢的工蜂；阿守是站在巢门口的那位：**查凭证、发钥匙、登记访客**。
它是这个后台的性格投射——权限不是拦路的墙，是有人认真守着门。

| 形象细节 | 对应功能 / 设定 |
|---|---|
| 胸前**六边形徽章**（对勾 / 金星 / 感叹号） | 蜂巢格 + 查验结果：普通 / 超管 / 告警 |
| 手里提着**一把钥匙** | 权限的授予与收回（对应 `admin_role` / `role_menu` / `role_dept` 三张连接表） |
| **翅膀**平时收在背后 | JWT 平时沉默；超管（旁路权限码）时才展开 |
| **尾针**平时收成小圆点 | 只在「仓库里检出密钥文件」时竖起 |
| 青蓝配色只出现在翅膀与徽章 | 与管理后台界面同色系的点缀，不喧宾夺主 |

**五种状态**（与框架宠物同一套判定机制，由项目状态自动切换；形象由 `docs/pet/render.mjs` 统一渲染）：

![阿守的五种状态：值守 / 查证 / 超管 / 打盹 / 竖针](docs/pet/keeper-states.png)

| 状态 | 表情 | 含义 | 触发条件 |
|:--:|:--:|---|---|
| 值守 | `(⌐■_■)` | 门岗正常，权限表与菜单树一致 | 测试全绿 · 无未提交改动 |
| 查证 | `(・_・)?` | 有变更待验证，凭证拿来 | 有未提交改动待审 |
| 超管 | `(＾▽＾)` | 超管入场，旁路所有权限码 | 在 main · 刚打 tag |
| 打盹 | `(-_-)zZ` | 这个点没人来访，登录记录空着 | 深夜 · 无人来访 |
| 竖针 | `(#°益°)` | 尾针竖起来了——有人把密钥写进仓库了！ | 仓库里检出密钥文件 |

## 项目宠物表情形态 · 阿守

![阿守的 12 种表情形态：笑 / 欢乐 / 哭 / 沮丧 / 惊恐 / 愤怒 / 撒娇 / 卖萌 / 讨好 / 敲打 / 捂脸 / 吐舌头](docs/pet/keeper-expressions.png)

| 名称 | 短码 | 名称 | 短码 |
|:--:|---|---|:--:|
| 笑 | `:emoji-smile:` | 欢乐 | `:emoji-joy:` |
| 哭 | `:emoji-cry:` | 沮丧 | `:emoji-depressed:` |
| 惊恐 | `:emoji-terrified:` | 愤怒 | `:emoji-angry:` |
| 撒娇 | `:emoji-coquettish:` | 卖萌 | `:emoji-cute:` |
| 讨好 | `:emoji-pleasing:` | 敲打 | `:emoji-knocking:` |
| 捂脸 | `:emoji-facepalm:` | 吐舌头 | `:emoji-tongue:` |

> 12 种彩色动态形态：笑 / 欢乐 / 哭 / 沮丧 / 惊恐 / 愤怒 / 撒娇 / 卖萌 / 讨好 / 敲打 / 捂脸 / 吐舌头。每个 SVG 自带表情、情绪特效与整体动效，并遵循 `prefers-reduced-motion`。

## 视觉资产授权 / Visual Assets License

本项目的项目宠物、图片、图标、表情（包括 `docs/pet-expressions/` 等目录下的
SVG / PNG）**不是开源资产**，版权归作者所有。未经书面许可不得复制、修改、再分发或
用于商业用途；商业使用需取得商业授权，联系 **erik@erik.xyz**。详见
[ASSETS-LICENSE.md](ASSETS-LICENSE.md)。

> 代码部分仍遵循仓库原有 `LICENSE`；第三方素材（如明确标注的 Go Gopher 等）仍遵循其原许可证。


## 项目介绍

**BRA（Bee Rust Admin）** 是一个开箱即用的后台管理系统：服务端用本仓库的
[bee-rust](https://github.com/erikwang2013/bee-rust) 框架写就（axum 路由 + 自研 ORM 执行层 +
INI 配置），前端是 Vite + React 18 + Ant Design 5 单页应用，两者通过一套统一的
`{code, msg, data}` 接口契约对接（错误响应另带稳定的业务错误码 `err` 与参数 `args`，前端据此出多语言文案）。

它解决的问题很实际：**权限要细到按钮、数据要看得到范围、登录要留痕**。因此：

- **权限模型是菜单 + 按钮两级**：角色勾选到按钮级权限码（如 `system:admin:resetPwd`），
  前端据此渲染菜单与按钮，后端**每个接口再校验一次**——前端隐藏只是体验，不是防线。
- **数据权限有五档**：全部 / 本部门及以下 / 本部门 / 仅本人 / 自定义（勾选部门），
  在列表查询层做条件注入，参数化绑定，不拼接用户输入。
- **会话无状态但有强制下线**：JWT 携带 `token_version`，改密 / 禁用 / 登出即失效旧令牌。
- **登录成功 / 失败 / 退出全部留痕**：IP、User-Agent、失败原因入库可查。

服务端启动时自动完成建表与种子（超管 + 菜单权限树），**不需要手工执行 SQL 迁移**。

## 架构设计

![架构设计](docs/diagrams/architecture.svg)

五层结构，自上而下：

| 层 | 组成 | 说明 |
|---|---|---|
| 客户端 | 浏览器 SPA | React 18 + TypeScript + Ant Design 5（Vite 构建） |
| 接入层 | nginx :8081 | 提供前端静态资源，`/api/` 反向代理到后端并透传 `X-Real-IP` |
| 应用层 | `bee_admin` | bee_router（axum）· Auth 提取器 · 93 个 API handler · datascope 数据权限 · 统一响应信封 |
| 框架层 | bee_orm / bee_config / bee_logs | 多后端连接池、QuerySet、CRUD、M2M、migrate 迁移；INI 配置；tracing 日志 |
| 数据层 | MySQL 8.4 | 22 张表（17 张业务表 + 4 张关系表 + 1 张明文备份表） |

## 功能设计

![功能设计](docs/diagrams/features.svg)

**权限模型是多对多的三段式**：管理员 ←`admin_role`→ 角色 ←`role_menu`→ 菜单与按钮；
角色 ←`role_dept`→ 部门（用于「自定义」数据范围）。超管（`is_super`）旁路所有权限码校验。

**数据权限五档**在列表接口注入查询条件：多角色的部门集合取并集，任一角色为「全部」则不加条件，
一个启用的角色都没有则落到 `WHERE 1 = 0`（什么都不给看，而不是全给看）。

## 请求周期

![请求周期](docs/diagrams/request-cycle.svg)

以 `GET /api/v1/admins` 为例的 12 步链路：浏览器 → nginx → bee_router → Auth 提取器
（校验签名 / `ver` / 账号状态，加载角色与权限码）→ handler（先 `require(权限码)`，再解析数据权限）
→ bee_orm（预编译 + 参数绑定）→ MySQL → 统一信封返回。**401（未登录）与 403（无权限）在查库之前就短路**。

## 服务生命周期

![服务生命周期](docs/diagrams/lifecycle.svg)

启动路径：读配置（JWT secret 少于 32 字符或为 `changeme` → **拒绝启动**）→ 连接 MySQL →
`migrate::sync` 同步表结构（**只建表 / 加列，绝不删列改类型**，模型加字段下次启动自动补列；
唯一键与索引由 `seed/schema.rs` 的裸 DDL 建 —— 上游的模型属性只表达列、主键与外键，没有 unique/index）→
首次启动（`admin` 表为空）写入超管，并**每次启动都幂等补齐**内置菜单与定时任务（按权限码查缺补漏；
**不是事务**——中途失败下次启动会接着补，不会留下半套）→ 监听 `127.0.0.1:8080` → 运行 →
收到 `SIGTERM` 退出（systemd 策略 `on-failure` 自动重启）。

## 功能介绍

| 模块 | 能力 |
|---|---|
| **登录鉴权** | 账号密码登录（argon2id）、JWT 签发与校验、登出、个人中心改密（改后强制重新登录） |
| **动态验证码与登录设备** | 可选的第二步验证（TOTP，RFC 6238，6 位 / 30 秒 / 容前后各一步）：绑定要现码确认、密钥 AES-256-GCM 加密落库（导出与日志里都没有明文）、解绑连密钥与备用码一起清；登录时**密码验过之后**才提示要码（先亮框等于泄露「该账号开了验证码」）。**备用码**：个人中心凭当前 TOTP 生成 10 枚（10 位、剔除易混字符），明文只回一次、库里只存 SHA-256(pepper‖code) 哈希，每枚只能用一次，重新生成旧码全部作废；登录输入 6 位纯数字走 TOTP、其余走备用码，未命中同样计入失败次数。个人中心可看**登录设备**列表（按 UA 归组、标出当前这台）并**踢掉单台**；**生成备用码 / 解绑**要当场输入当前码确认，连续输错 5 次后 5 分钟内一律 429（进程内 per-admin 限流，多实例不共享）。验证器与备用码都丢了由有库权限的运维清 totp 列（`UPDATE admin SET totp_secret='', totp_enabled=0, totp_last_step=NULL WHERE id=<id>;`，并删备用码 `DELETE FROM admin_backup_code WHERE admin_id=<id>;`）后重新登录再绑，或恢复备份（重置密码不解除绑定） |
| **通知渠道** | 出站通知两种：**Webhook**（事件 `notice.published` 公告发布 / `account.locked` 账号锁定 → POST JSON `{event,at,data}`；可选 **HMAC-SHA256 签名**，头 `X-BRA-Timestamp` + `X-BRA-Signature: sha256=…`；保存时拒绝链路本地 / 未指定 IP 字面量，**发送时解析域名并再次封禁**，回环 `127.0.0.1` 放行；**不跟随重定向**（3xx 按投递失败处理，防止 302/307 绕过封禁）；失败落 `notify_delivery` 账本，`notify_retry` 每 5 分钟按 1/5/15 分钟退避重投、最多 3 次，账本留 30 天）与 **SMTP**（配置页可发一封测试邮件到**自己**的邮箱，失败回原因；另可开启**公告邮件推送**：发布公告时给全部启用且填了邮箱的账号各发一封，**发布者本人除外**，默认关）。投递异步（业务响应不等它），配置存系统配置的 `notify.*` 键（一律不公开、改完不用重启）；SMTP 口令与 Webhook 签名密钥加密存、接口只回「是否已设置」，通用配置页看不见也改不动它们 |
| **仪表盘** | 首页四类计数（管理员 / 角色 / 部门 / 公告）+ 近 7 天登录与失败登录趋势（手绘 SVG 迷你柱状图，无图表库），登录即可、不挂权限码；无数据的日期补 0，全 0 显示空态 |
| **管理员管理** | 分页列表（用户名 / 状态 / 部门筛选）、增删改、分配角色、重置密码、启用禁用（禁用即踢下线） |
| **角色管理** | 增删改查、菜单权限树勾选、数据范围五档、自定义部门、删除前校验是否被管理员引用 |
| **菜单管理** | 树形目录 / 菜单 / 按钮三层、权限码、图标与排序、防环校验、删除前校验子节点与角色引用 |
| **部门管理** | 树形部门、负责人与排序、删除前校验子部门与在编管理员 |
| **登录记录** | 按用户名 / 状态 / 时间段分页查询、按条件清空，记录受数据权限约束 |
| **操作日志** | 有副作用的操作（POST / PUT / DELETE）自动留痕（时间 / 操作人 / 模块 / 动作 / 请求 / IP）、按用户名 / 模块 / 时间段筛选、导出与按条件清空（清空本身也留痕），记录受数据权限约束。管理员 / 角色 / 部门 / 字典四类编辑额外记 **变更摘要**（哪些字段 旧→新）；账号的邮箱 / 手机号是加密列，摘要只记「已变更」不落值 |
| **字典管理** | 字典类型 + 字典项两级维护、同类型内 value 唯一、删除类型级联删项、CSV 导出；`GET /dicts/{code}/items` 是登录即可用的下拉数据源（只回启用项） |
| **数据导入** | 管理员 / 部门 / 字典项三个页面：下载模板（表头即契约）→ 上传 CSV → 逐行报告（行号 + 原因）。**单事务全有或全无**（有一行不合规整批不写库），每行都过发起人自己的权限与数据范围；只认 UTF-8、单次 5000 行 |
| **导出中心** | 异步导出：发起（操作日志 / 登录记录，与同步导出**共用同一套渲染**、产物逐字节一致）→ 定时任务执行（进程内并发上限 2，跨多次触发不叠加）→ 列表自动轮询（有进度列）→ 下载 / 取消 / 删除。两条权限闸（`system:export:list` + 该类型的查看码），**个人工作台**（只看得到自己的任务）；成品按代次命名，过期连记录带文件自动清理，手动删除任务也会清掉该任务的全部代次文件 |
| **定时任务** | **8 个内置任务**由**代码注册**（code 是注册键，库里只放调度与执行记录，界面不能新增/改名）、固定间隔（秒）调度 + 启停、手动触发一次、执行记录分页；`[job] enabled` 总开关关掉后仍可手动补跑 |
| **数据库备份** | 内置任务 `db_backup` 用 `mysqldump` 把整库导到磁盘（文件 0600、可配目录与保留份数，默认留 7 份），执行记录进 `job_log`；恢复是**手工动作**（停服 → `mysql < dump`，见 faq.md §5.10） |
| **通知公告** | 草稿 / 已发布两态（发布写 `published_at`，回退草稿不清空——留痕）、增删改、删公告同事务级联删已读记录与配图附件行（磁盘文件交附件孤儿清理）；顶栏铃铛显示未读数 |
| **附件中心** | 通用附件库：分片上传（白名单内 13 种扩展名、单文件 10MB、每人总量受 `[attach] quota_mb` 约束）、列表筛选、下载、删除，并可把附件**关联**到业务对象（v1 是公告配图，绑定带排序）；**下载要过鉴权**——绑到已发布公告的附件对所有登录用户可下，其余按上传人的数据权限；同一份内容在同一月内只落一份文件（文件名取内容 MD5、子目录按月），**删除只删行不删盘**（文件交内置任务 `attach_orphan_clean` 回收：5 分钟保护期、默认每小时一轮，间隔取库里那一行、升级不覆盖已有行）；公告编辑页有「配图」栏（上传即绑定） |
| **系统配置** | 键值对配置（键 / 值 / 类型 / 分组 / 备注 + 公开开关），分组表单编辑且只提交改动项（公开开关与值同一次保存）；标记「公开」的键经 `GET /configs/public`（免登录）供给登录页与页脚（副标题、版权、项目地址、页脚显隐），读不到时前端回落内置默认值；内置键由启动流程幂等补齐，改配置刷新即变 |
| **中英双语** | 界面文案中/英两套（顶栏切换，跟随浏览器语言，存 localStorage），antd 组件文案与日期格式跟着切；**后端不做多语言消息**——错误响应带稳定业务错误码 `err` + 参数 `args`，前端查表出文案，查不到回落后端中文 `msg`（任何情况都有话可说） |
| **站点信息** | 登录页与侧边栏底部一行：版本号（构建时注入，发布产物显示真实 tag、本地构建显示 `dev`）、版权、项目地址（链到仓库）—— 版权文字 / 项目地址 / 是否显示都读系统配置的公开键，报障带上版本号最省事 |

接口共 93 个（91 个业务接口 + `/api/v1/health` 探活 + `/api/v1/version` 版本），路径与字段的完整定义见
[设计文档 §5.3](docs/superpowers/specs/2026-10-05-bee-rust-admin-design.md)，
字典 / 定时任务 / 通知公告 / i18n 的契约另见 [C 组设计](docs/superpowers/plans/2026-10-06-bra-v1.5-c-modules.md)。

## 项目结构

```
crates/                 bee-rust 框架（workspace 成员，本后台直接依赖）
  bee_orm/              ★ ORM 执行层：多后端连接池 / QuerySet / CRUD / M2M / migrate
  bee_orm_macro/        ★ #[derive(Model)]：元数据 + 行映射 + 参数绑定代码生成
  bee_router/           axum 封装的路由
  bee_config/           INI 配置（含解析与监听）
  bee_logs/             tracing 日志初始化
  bee_cli/ bee_kv/ ...  其余框架 crate
admin/                  管理后台服务端（crate bee_admin）
  src/main.rs           启动：配置 → 连库 → migrate → 种子 → 路由 → 监听
  src/config.rs         INI 配置与启动校验（JWT secret 强度、DSN 覆盖）
  src/error.rs          ApiError + 统一信封（code/msg/err/args）+ 错误码表 + 请求体提取器
  src/auth.rs           JWT 签发校验 + Auth 提取器（权限码加载、token_version 校验）
  src/datascope.rs      数据权限解析与查询注入（部门子树 / 本人 / 自定义）
  src/api/v1/           admin · attach · audit_log · auth · avatar · config · dashboard · dept · dict ·
                        export · import · job · login_log · menu · notice · notify · role · totp
                        十八个模块目录（v1 入口为 api/v1/mod.rs；csv 是内部工具模块，不出接口）
  src/models/           15 个模型（migrate 建表）。列长度用 `sql_type = "VARCHAR(n)"` 声明；
                        **唯一键与索引不走模型**（上游属性没有这两个概念），集中在
                        `src/seed/schema.rs` 的 CONSTRAINTS 里建。连接表（admin_role/role_menu/
                        role_dept/notice_read）是复合主键、无模型结构体，走 seed 的裸 DDL，
                        读写见 src/relations.rs
  src/seed/             建表、连接表 DDL、首次种子（超管 + 菜单权限树）：schema / defaults / menus
  conf/                 app.conf（gitignore）· app.conf.example · app.conf.test
  deploy/               systemd 单元 + nginx 站点配置
  tests/                集成测试：起真实进程 + 连真库（含全链路）
  web/                  前端 SPA（React + antd，见下）
    src/api/            接口层（契约类型 + axios 拦截器 + 错误码→文案映射）
    src/i18n/           类型化词表（中英）+ I18nProvider + 语言开关
    src/auth/           AuthContext + 按钮级权限组件
    src/layouts/        动态菜单布局
    src/pages/          登录 / 首页 / 管理员 / 角色 / 菜单 / 部门 / 字典 / 定时任务 / 通知公告 /
                        系统配置 / 通知设置 / 导出中心 / 附件中心 / 登录记录 / 操作日志 / 个人中心
docs/
  install.md            ★ 安装说明（三种装法、配置生成步骤、常见坑）
  usage.md              ★ 使用手册（权限体系怎么落地、各模块怎么用）
  faq.md                ★ 常见问题与运维（usage 第 5 章单独成册：排障、备份恢复、清理明文备份）
  release-checklist.md  ★ 发布 / 升级检查清单（逐条打勾）
  release.md            发版怎么发（scripts/release.sh 用法、产物口径、发完怎么验）
  ponytail-debt.md      有意搁置台账（源码里 ponytail: 注释的汇总视图）
  scaffold.md           模块脚手架用法（生成物必先自编译过才落盘）
  diagrams/             本 README 引用的 4 张 SVG 图
  pet/                  项目宠物「阿守」的五态 SVG + 渲染器（node render.mjs）
  social/               社交预览图 1280×640（仓库 Settings → Social preview 上传那份 PNG）
  superpowers/specs/    设计文档（数据模型、接口清单、数据权限规则）
  superpowers/plans/    实施计划（ORM 执行层、后端、前端）
install.sh              ★ 一键安装脚本（拉发布产物 / 源码构建 → 生成配置 → 装 systemd + nginx → 自检）
scripts/
  lib/                  install.sh 的分步实现（install-*.sh 与共用函数；缺了 install.sh 直接报错）
  scaffold-module.sh    ★ 模块脚手架入口：一条命令生成 CRUD 模块骨架（实现拆在 scaffold-*.sh）
  release.sh            发版：构建 → strip → 打包 → 产物自检 → gh release create
  publish.sh            逐个发布工作区内未上传到 crates.io 的 crate
  smoke-install.sh      install.sh 冒烟验证（/tmp 里装一套 scratch 实例 + 升级一次 + 清理）
Dockerfile              后端镜像（多阶段构建）
docker-compose.yml      MySQL + 后端 + 前端一键起栈
```

> `install.sql` 在仓库根目录，但**与本项目无关** —— 那是另一个工具的记忆库 schema，
> 安装 BRA 不需要也不使用它，别被名字骗到。

## 使用说明

> **想快点装起来？** 看 [`docs/install.md`](docs/install.md) —— 三种装法的完整命令、
> 生成密钥的命令、一份可直接改的 `app.conf`，以及常见坑。命令行党直接：
>
> ```bash
> git clone --depth 1 https://github.com/erikwang2013/bee-rust-admin.git
> cd bee-rust-admin
> less install.sh && bash install.sh      # 交互式；--help 看全部参数
> ```
>
> `install.sh` 只是入口，具体步骤在同仓库的 `scripts/lib/install-*.sh` —— **旧版「只下载一个
> install.sh」的用法已退役**，必须先克隆完整仓库再跑（缺 `scripts/lib/` 会直接报错退出）。
>
> **用起来**之后的手册在 [`docs/usage.md`](docs/usage.md)（权限体系怎么落地、各模块怎么操作）；
> 排障与运维（忘记密码、任务没跑、备份恢复）在 [`docs/faq.md`](docs/faq.md)。

### 环境要求

| 组件 | 版本 |
|---|---|
| Rust | 1.99+（edition 2024） |
| MySQL | 8.0+（开发于 8.4，utf8mb4） |
| Node / pnpm | Node 20+ / pnpm 9+ |
| nginx | 任意近期版本（仅生产部署需要） |

> ⚠️ **用预编译产物（`install.sh` 的默认模式）需要 glibc ≥ 2.38**（Debian 13 / Ubuntu 24.04+）。
> Ubuntu 22.04 / Debian 12 及更老的发行版请用 `install.sh --mode build` 或 Docker ——
> 脚本会自己检测，跑不起来会在**写盘之前**退出并给出这两条替代命令。

### 快速开始

**1. 建库**（表由服务启动时自动创建，无需手工建表）

```sql
CREATE DATABASE bee_admin DEFAULT CHARSET utf8mb4 COLLATE utf8mb4_0900_ai_ci;
```

**2. 配置**

```bash
cp admin/conf/app.conf.example admin/conf/app.conf
# 必填三项：
#   [db] dsn                              MySQL 账号密码
#   [jwt] secret                          ≥32 字符随机串，且不能是 changeme
#   [app] encrypt_key                     base64:<32 字节>，生成：openssl rand -base64 32 | sed 's/^/base64:/'
# 建议两项（部署间必须不同）：
#   [app] hashids_salt                    对外 id 短串的盐
#   [app] snowflake_worker / snowflake_dc 多实例各进程必须不同，否则撞号
```

> ⚠️ `encrypt_key` **缺失时服务拒绝启动**（刻意的：不允许「以为加密了其实没加密」地跑起来），
> 且它**不可更换** —— 换了密钥，库里已加密的邮箱/手机号就解不开了。
> 同理 `hashids_salt` 一换，已发出去的短串 id 全部作废。

其余可选开关见示例文件：`[log] retain_days`（日志保留天数，`0` = 永久，同时作用于
登录/操作/任务执行记录）、`[job] enabled`（定时任务总开关，关掉后仍可手动补跑）、
`[auth] max_fail` / `lock_minutes`（登录失败锁定）、`[auth] captcha`（登录图形验证码，
默认开）、`[auth] captcha_trajectory`（滑块拖拽轨迹校验，**默认关**）。
`captcha_trajectory` 开启后滑块答案必须带轨迹（插件默认阈值：≥4 个采样点、300–5000ms、
线性度 ≤0.99），只交裸数值的客户端一律失败 —— 新前端总是上报轨迹，开它不用重发前端；
代价是「极快甩动」（<300ms）会被判机器（症状：滑块怎么拖都提示「验证码错误」），
要放宽得改插件侧的 `min_duration_ms`。

> 升级已有部署时：验证码凭据由新前端提交。**若先换后端、暂不重建前端，先把
> `[auth] captcha` 写成 `false`**，否则老前端的登录会被判 400（验证码错误）。

**3. 起服务**（首次启动自动建表、写入超管与菜单权限种子）

```bash
cargo run -p bee_admin
curl http://127.0.0.1:8080/api/v1/health    # => OK
curl http://127.0.0.1:8080/api/v1/version   # => {"version":"dev"}（构建时注入，见 release.sh 的 BRA_VERSION）
```

**4. 起前端**

```bash
cd admin/web && pnpm i && pnpm dev
# 打开 http://127.0.0.1:5173（vite 已把 /api 代理到 127.0.0.1:8080）
```

也可以用生产构建在本机预览（端口与线上 nginx 一致，同样反代 `/api`）：

```bash
cd admin/web && pnpm build && pnpm preview   # http://localhost:8081
```

### 初始账号

| 账号 | 密码 |
|---|---|
| `admin` | `admin/conf/app.conf` 的 `[seed] initial_admin_password`（示例配置为 `change-me-please-1`） |

超管仅在首次启动（`admin` 表为空）时写入。**登录后请立即到「个人中心」修改密码**；
生产环境务必同时改掉 `initial_admin_password` 与 `[jwt] secret`。

> 提示：`BEE_ADMIN_CONF` 可指定配置文件路径；`BEE_ADMIN_DB_DSN` 覆盖数据库连接串；
> `BEE_ADMIN_HTTP_ADDR` 覆盖监听地址（容器里绑 `0.0.0.0:8080` 时必须用）。

### 请求安全扫描（security-rust）

内置 32 个攻击特征检测器（SQLi / 命令注入 / SSRF / XXE / Log4Shell / 路径穿越 …），
默认**只报告不拦截**：

```ini
[security]
# off（默认）：命中只留痕（WARN 日志 + 一条审计，记等级与检测器名，不记原始 payload）
# high / critical：达到该等级直接 403（err = security.blocked）
scan = off
```

误报是真实存在的（密码里带 `' or 1=1--` 的用户、公告里贴的代码片段），所以默认不拦。
**要开 `high` 之前先知道两件事：**

- **XSS 拦不住**：XSS 在插件的评分里只有 Low，而阈值只有 off/high/critical ——
  能检出、能留痕，但不会被 403
- 扫描**扫的是原始串 + 解码后**两份（攻击载荷基本都是百分号编码的，
  只扫原文等于漏掉绝大多数注入）

扫 URL、指定请求头（`host` / `user-agent` / `referer` / `origin` / `x-forwarded-host`）
与 ≤64KB 的 JSON 体（**读完原样放回**）。**multipart 一律不碰**（碰了会把整个上传
缓冲进内存，毁掉头像的分片上传）。

> `referer` / `origin` 只扫 `scheme://authority` **之后**的部分 —— 浏览器自动带的同源
> Referer 若是环回/内网 IP，插件的 SSRF 检测器判 **Critical**，照字面扫会让
> `scan = high` 在按 IP 访问的部署上把**每个**正常请求都 403。

### 接口文档站（apidoc-rust）

后台内置 apidoc-rust 的接口文档站，**默认关闭**（文档页会把整个后台的接口面、请求/响应
结构都摊开，它是开发/交付工具，不该跟着生产实例一起对外）。要开就在 `app.conf` 里写：

```ini
[apidoc]
enabled = true
password = <自己定一个口令>
```

- `enabled = true` 而 `password` 空着 → **拒绝启动**（与 `encrypt_key` 一个态度：不允许
  「以为有口令其实没有」地跑起来）；非法布尔值同样拒绝启动
- 挂载点：`/apidoc`（UI，浏览器直接开）、`/apidoc/api.json`（原始接口清单，93 个接口）
- 文档站**不在审计 / 安全扫描层的覆盖范围内**（它 merge 在全部中间件之后，读文档不是业务
  操作，记进操作日志只是噪音），整棵树也在 `/api/v1` 之外，与业务接口互不影响

数据路由（`api.json` / `export` / `mock` / `share` / `generate`）要带 token：**POST 口令换
token**（口令走请求体，不进 URL），再把 token 带在 query 上：

```bash
TOKEN=$(curl -s -X POST http://127.0.0.1:8080/apidoc/auth \
  -H 'Content-Type: application/json' -d '{"password":"你的口令"}' \
  | sed 's/.*"token":"\([^"]*\)".*/\1/')
curl -G --data-urlencode "token=$TOKEN" http://127.0.0.1:8080/apidoc/api.json
```

`/apidoc`（UI 页本身）不设守卫，口令只在取 token 时校验 —— 别把 UI 当成「有密码的站点」。
登录失败有退避（前 3 次不罚，之后 1→2→…封顶 10 秒）；token 默认 24 小时有效，容器/进程重启
默认换一把随机 `secret_key`（旧 token 立即失效，多实例想共用令牌得自己配 `secret_key`）。

> ⚠️ **token 带在 query 上，等价于一个可重放的通行证** —— URL 在访问日志 / 浏览器历史 /
> 跳板机记录里留一次，抓到的人到期前就都能用。所以**只在可信网络（内网 / 跳板机）里开**，
> 别对着公网。

### Docker 一键部署

仓库里已备好容器化配置：

| 文件 | 作用 |
|---|---|
| `Dockerfile` | 后端镜像：多阶段构建（Rust 编译 → Debian 运行），非 root 用户运行 |
| `admin/web/Dockerfile` | 前端镜像：pnpm 构建 → nginx，`/api/` 上游由环境变量 `BACKEND_UPSTREAM` 注入 |
| `docker-compose.yml` | MySQL + 后端 + 前端一键起栈，对外 `8081` |
| `compose-up.sh` | compose 一键起栈：缺的口令 / 密钥自动生成并写回本地文件（见下） |
| `deploy/Dockerfile.prebuilt` | 后端镜像的**预编译版**：打 `target/release-assets/bee_admin`，不发版时用不到（见下） |

```bash
./compose-up.sh --build     # 缺的口令/密钥（MySQL root、encrypt_key、jwt secret、初始口令）
                            # 自动生成并写回 .env 与 admin/conf/app.conf；已有镜像时去掉 --build
# 打开 http://localhost:8081（初始账号 admin；初始口令生成时会打印在屏幕上，
# 也在 admin/conf/app.conf 的 [seed] initial_admin_password）

# 想自己填（或不想用脚本）：cp .env.example .env && cp admin/conf/app.conf.example admin/conf/app.conf
# 填好 MySQL root 密码、[jwt] secret、[app] encrypt_key（示例值会被拒绝启动）再 docker compose up -d --build
```

- 容器内的数据库地址由 `BEE_ADMIN_DB_DSN` 注入（指向 compose 里的 `mysql` 服务）；
  配置文件仍挂载自 `admin/conf/app.conf`，JWT secret 与初始密码从那里读
- 后端容器以**宿主机配置文件属主**的身份运行（`APP_UID`/`APP_GID`，默认 `1000:1000`）：
  `app.conf` 通常是 600 权限，用镜像默认的非 root 用户会读不到配置而启动失败；
  宿主 uid 不是 1000 时在 `.env` 里改
- 上传的头像落在容器内 `/home/uploads`（镜像把 WORKDIR 指到 `/home`，并预置了这个可写目录：
  `upload_dir=uploads` 是相对 WORKDIR 的路径，而 `/app` 属 root，以 `APP_UID` 跑写不进去）；
  **容器重建即丢**
- `compose-up.sh` 只补「空 / 占位符」的密钥口令，填过的一律不动（重复跑不会重新生成）；
  生成的值落在 `.env` 与 `admin/conf/app.conf` 里 —— **记得备份 `admin/conf/app.conf`**：
  `encrypt_key` 丢了（或换了），库里已加密的 email/phone 永久解不开
- 想接已有的 MySQL：删掉 compose 里的 `mysql` 服务与 `depends_on`，直接改 `BEE_ADMIN_DB_DSN`
- 本机已有 `bra-backend`/`bra-web` 镜像时（比如下面离线包 `docker load` 进来的）
  不用 `--build`：`docker compose up -d` 直接用现有镜像，`--build` 才现场构建
- 只构建镜像（不启动）：`docker build -t bra-backend .`（后端）与
  `docker build -t bra-web admin/web`（前端）；发布脚本走的是预编译版
  `docker build -f deploy/Dockerfile.prebuilt -t bra-backend:<tag> .` —— 直接打
  `target/release-assets/bee_admin`（发布口径 strip 过的那个），不重编译，~30 秒
- **发布产物里的离线包**：每次 release 附带 `bra-images-vX.Y.Z.tar.gz`（后端 + 前端
  镜像，`docker load -i` 直接导进来）与 `bra-compose-vX.Y.Z.tar.gz`（compose 文件 +
  预填 `.env` + `app.conf.example` + 一页说明）—— 解包、`docker load`、按说明改两处
  口令，再 `docker compose up -d` 即可起栈（离线部署用那套，见 `docs/release.md`）
- 镜像只在本机构建、离线分发（`docker save`/`load`，或 `docker tag` + `docker push`
  推到自己的仓库），**不经过 GitHub CI 发布**；发版时由 `scripts/release.sh` 把两个镜像
  打进上面的离线包一起发（本机没有 docker daemon 时用 `--skip-images` 跳过）

### 测试

```bash
# 框架：单测 + 真库集成（未设 DSN 的集成测试跳过并打印原因）
cargo test -p bee_orm
BEE_ORM_TEST_DSN='mysql://user:pass@127.0.0.1:3306/bee_orm_test' cargo test -p bee_orm -- --nocapture

# 后台：单测（配置 / JWT / 数据权限 / 防环等纯函数）
cargo test -p bee_admin --bins

# 后台：整包（单测 + 全部真库集成套件；不逐个列 --test，避免新套件被漏在 CI 外）
# 注意：会先 DROP 测试库所有表再重建，只对测试库执行
BEE_ADMIN_DB_DSN='mysql://user:pass@127.0.0.1:3306/bee_admin_test' \
  cargo test -p bee_admin -- --nocapture
```

`BEE_ADMIN_DB_DSN` 会覆盖配置里的 `[db] dsn`（便于测试注入凭据），因此测试库账号不必写进仓库；
集成测试带**生产库护栏**——DSN 指向的库名不以 `_test` 结尾时直接拒绝执行。

### 部署

systemd 托管服务（监听 `127.0.0.1:8080`）+ nginx 对外 8081：静态资源指向前端构建产物
`admin/web/dist`，`/api/` 反向代理到后端并透传 `X-Real-IP`（登录记录需要真实 IP）。

```bash
cargo build --release -p bee_admin
cd admin/web && pnpm build

sudo cp admin/deploy/bee-admin.service /etc/systemd/system/
sudo systemctl daemon-reload && sudo systemctl enable --now bee-admin

sudo cp admin/deploy/nginx.conf.example /usr/local/nginx/conf/vhost/bee-admin.conf
sudo nginx -t && sudo systemctl reload nginx
```

文件内的注释写了各自的安装位置与前置条件；设计文档 §7 有部署方案的完整说明。

> 没有 root 的机器可换用**用户级单元** `admin/deploy/bee-admin.user.service`：装到
> `~/.config/systemd/user/`、命令全带 `--user`，能力与系统级等价；开机自启需该用户开过
> linger（`loginctl show-user <用户> -p Linger`）——安装与管理细节见
> [安装文档 (c)](docs/install.md#c-手工部署)。

> 这两份文件是**手工路径的示例**，里面的路径与用户名写死成本机的
> （`/home/wwwroot/bee-rust-admin`、`erik`），照着用要自己改。
> **想省掉抄改就用 [`install.sh`](install.sh)**：它按实际的 `--prefix` / 用户 / 端口
> 渲染这两份文件，顺带生成配置、建库、跑自检。完整说明见 [`docs/install.md`](docs/install.md)
> 里的「一键脚本」一节。
> **升级现有部署**前先过一遍 [`docs/release-checklist.md`](docs/release-checklist.md)
> （配置键、前后端同版本、回滚一条 `mv`）。
> **要发版**看 [`docs/release.md`](docs/release.md)：一条 `bash scripts/release.sh vX.Y.Z --notes-file …`
> 包掉构建、strip、打包、自检与 `gh release create`（`--dry-run` 只验不发）。

## 版权

© 2026 erik · [https://erik.xyz](https://erik.xyz)

本项目基于 [Apache-2.0](LICENSE) 许可发布。
