/** 统一响应信封。C3 起错误响应可能多带 `err`/`args`（见 i18n 的错误码表）。 */
export interface ApiResult<T> {
  code: number;
  /** 永远保留的中文原文（日志、curl、下载客户端都还要用） */
  msg: string;
  /** 稳定的业务错误码，如 `auth.bad_credentials`；**没有稳定码时整个字段不带**（不是 null） */
  err?: string;
  /** 错误码的原始参数（未格式化），只有带参的码才有 */
  args?: Record<string, unknown>;
  data: T;
}
export interface Page<T> { list: T[]; total: number }

export interface UserInfo {
  id: string; username: string; nickname: string; avatar: string; is_super: boolean; dept_id: string;
  /** 后端 /auth/profile 返回时回填资料表单；未返回则为空 */
  email?: string; phone?: string;
  /** 是否已绑定动态验证码（项 7）；登录/资料接口都带 */
  totp_enabled?: boolean;
  /** 未使用的备用码数量（F4-1）；资料接口带，个人中心展示 */
  backup_codes_left?: number;
}

/** 登录设备（项 7，`/auth/devices` 的每一行）。 */
export interface LoginDevice {
  id: string; user_agent: string; ip: string; created_at: string; last_active_at: string;
  /** 本次请求来自这台设备（列表里唯一一行 true） */
  current: boolean;
}

/** 动态验证码绑定材料（`/auth/totp/setup`）：密钥、otpauth 链接、二维码 SVG。 */
export interface TotpSetup { secret: string; otpauth_url: string; qr_svg: string }
export interface Profile { user: UserInfo; roles: string[]; perms: string[] }
export interface MenuNode { id: string; parent_id: string; name: string; path: string; icon: string; children?: MenuNode[] }

/** 点击验证码的提示项：`order` 是要点击的顺序，`text` 是目标文字（icon 模式下另带缩略图）。 */
export interface CaptchaTarget { text: string; order: number; thumb?: string }
export interface CaptchaExtra {
  /** click 有 */
  texts?: CaptchaTarget[];
  /** slider 有：拼图块 PNG 的 data URI（本体尺寸是 puzzle_w × puzzle_h，jigsaw 下外扩后更大） */
  puzzle?: string;
  puzzle_w?: number;
  puzzle_h?: number;
}
/**
 * 图形验证码（poster-rust `CaptchaResult`）。`type` 决定 `extra` 的形状与交互方式；
 * **答案不在响应里**（只在服务端存储中），前端提交的是用户操作的结果。
 */
export interface CaptchaData {
  key: string;
  /** PNG 的 data URI，直接塞 `<img src>` */
  image: string;
  type: 'click' | 'rotate' | 'slider';
  extra: CaptchaExtra;
}
/** 提交给后端的答案：poster-rust `Answer` 的 serde 表示（外部标签，见 `captcha.rs`）。 */
export type CaptchaAnswer =
  | { Click: [number, number][] }
  | { Rotate: number }
  | { Slider: number }
  /** 带拖拽轨迹的滑块（`[auth] captcha_trajectory` 开启时插件才校验轨迹，见 login/captcha.ts） */
  | { SliderWithTrail: { x: number; trail: [number, number, number][]; duration_ms: number } };

export interface Admin {
  id: string; username: string; nickname: string; email: string; phone: string; sex: number;
  dept_id: string; dept_name?: string; status: number; is_super: boolean; remark: string;
  created_at: string; role_ids?: string[]; role_names?: string[];
}
export interface Role {
  id: string; name: string; code: string; sort: number; data_scope: number;
  status: number; remark: string; created_at: string;
  /**
   * 当前操作者能不能把这个角色授出去（后端角色列表/详情带的权威标记）。
   * 前端只认这个布尔值，不自己算 data_scope / 权限码；保存时后端还会硬校验一次。
   */
  grantable?: boolean;
}
export interface Menu {
  id: string; parent_id: string; name: string; type: 'M' | 'C' | 'F'; perm: string;
  path: string; component: string; icon: string; sort: number; visible: number;
  status: number; children?: Menu[];
}
export interface Dept {
  id: string; parent_id: string; name: string; sort: number; leader: string;
  phone: string; status: number; children?: Dept[];
}
export interface LoginLog {
  id: string; username: string; ip: string; user_agent: string;
  status: number; msg: string; created_at: string;
}
export interface AuditLog {
  id: string; admin_id: string; username: string; module: string; action: string;
  method: string; path: string; status: number; msg: string;
  /** 变更摘要 JSON 串（项 8）：`{"字段":{"old":…,"new":…}}`；没有登记摘要的路径为空串 */
  changes: string;
  duration_ms: number; ip: string; created_at: string;
}
export interface DictType {
  id: string; name: string; code: string; status: number; remark: string; created_at: string;
}
export interface DictItem {
  id: string; type_code: string; label: string; value: string; sort: number;
  status: number; remark: string; created_at: string;
}
export interface Job {
  id: string; name: string; code: string;
  /** 间隔秒数的**字符串**（列是 varchar64）：列名保留 cron 是为了将来换表达式时不改列名 */
  cron: string; status: number;
  /** 从未跑过时为空 */
  last_run_at: string | null; last_status: number | null; last_msg: string;
  created_at: string; updated_at: string;
}
export interface SysConfig {
  id: string; key: string; value: string;
  /** string 单行 / text 多行 / bool 开关（值都存字符串，"true"/"false"） */
  value_type: 'string' | 'text' | 'bool';
  group: string; is_public: number; remark: string;
  created_at: string; updated_at: string;
}
/** 公开配置（`/configs/public`，免登录）：键 → 值；bool 类型已是 JSON 布尔 */
export type PublicConfig = Record<string, string | boolean>;

/** 通知渠道配置（项 6）：**没有 `smtp_password` 字段** —— 口令只进不出，只回是否已设置 */
export interface NotifyConfig {
  webhook_enabled: boolean; webhook_url: string;
  /** 是否已配置签名密钥（F4-2）：密钥本身只进不出 */
  webhook_secret_set: boolean;
  smtp_enabled: boolean; smtp_host: string; smtp_port: number;
  smtp_user: string; smtp_from: string;
  smtp_encryption: 'starttls' | 'ssl' | 'none';
  /** 公告发布时给全体启用账号发邮件（发布者本人除外）；SMTP 开关关着时不发 */
  email_notice_enabled: boolean;
  smtp_password_set: boolean;
}

/** 按天的登录计数（仪表盘趋势）：`date` 为 YYYY-MM-DD，旧→新 */
export interface LoginDay { date: string; success: number; fail: number }
export interface DashboardStats {
  counts: { admins: number; roles: number; depts: number; notices: number };
  logins: { success_total: number; fail_total: number; days: LoginDay[] };
}
export interface JobLog {
  id: string; job_code: string; started_at: string; duration_ms: number;
  status: number; msg: string;
}
/** 手动触发：**后台执行**，接口只回「已触发」（跑完的结果去任务列表/执行记录看） */
export interface JobRunResult { triggered: boolean }

export interface Notice {
  id: string; title: string; content: string;
  /** 0 草稿 / 1 已发布。取消发布不清 published_at（留痕：曾发布过） */
  status: number; created_by: string; published_at: string | null;
  created_at: string; updated_at: string;
}
/** 未读接口：total 是未读总数（铃铛角标用它），list 只给最近 50 条 */
export interface NoticeUnread { total: number; list: Notice[] }

/** CSV 导入（项 3）：三种导入共用一套逐行报告。 */
export type ImportKind = 'admin' | 'dept' | 'dict-item';
export interface ImportRow {
  /** 文件行号（表头是第 1 行） */
  line: number;
  ok: boolean;
  /** 不通过的原因；通过时为空串 */
  msg: string;
}
export interface ImportReport {
  kind: ImportKind;
  /** 数据行数（不含表头与空行） */
  total: number;
  /** 实际写入条数：全对 = total，有坏行 = 0 */
  created: number;
  rows: ImportRow[];
}

/** 附件（项 5）。`stored_path` 不下发（后端 skip），下载一律走 `/attachments/{id}`。 */
export interface AttachmentRow {
  id: string; name: string; ext: string; mime: string; size: number; sha256: string;
  admin_id: string; admin_name: string;
  /** 关联类型：空串 = 未绑定（目前只有 notice） */
  entity_type: string;
  /** 关联对象 id（短串）；未绑定是空串 */
  entity_id: string;
  /** 关联对象的标题（列表里给眼睛看的那一列） */
  entity_label: string;
  /** 绑定了**已发布**公告 = 所有登录用户都能下 */
  is_public: boolean;
  sort: number; created_at: string;
}

/** 异步导出（项 4）：类型在**后端代码**里注册（能往库里写导出语句就是任意读库），前端只列出来。 */
export type ExportKind = 'audit-log' | 'login-log';
export interface ExportTask {
  id: string; admin_id: string; username: string; kind: string;
  /** 筛选条件 JSON 字符串：`{}` = 全量 */
  params: string;
  /** 0 待处理 / 1 执行中 / 2 已完成 / 3 失败 / 4 已取消 */
  status: number;
  row_count: number; file_size: number;
  /** 已写进文件的行数：执行中每批（5000 行）回写一次；取消的那行停在取消时的部分值 */
  progress_rows: number;
  /** 失败原因 / 完成摘要 */
  msg: string;
  created_at: string; started_at: string | null; finished_at: string | null;
  /** 心跳：每批刷新一次（后端判「任务是不是还活着」用，列表不显示） */
  heartbeat_at: string | null;
  /** 过期时刻：清理任务按它连记录带文件一起删 */
  expire_at: string;
}
