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
}
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
export type CaptchaAnswer = { Click: [number, number][] } | { Rotate: number } | { Slider: number };

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
export interface JobLog {
  id: string; job_code: string; started_at: string; duration_ms: number;
  status: number; msg: string;
}
/** 手动触发（同步执行）的结果：status 1 成功 / 0 失败，msg 就是写进 job_log 的那条 */
export interface JobRunResult { status: number; msg: string; duration_ms: number }

export interface Notice {
  id: string; title: string; content: string;
  /** 0 草稿 / 1 已发布。取消发布不清 published_at（留痕：曾发布过） */
  status: number; created_by: string; published_at: string | null;
  created_at: string; updated_at: string;
}
/** 未读接口：total 是未读总数（铃铛角标用它），list 只给最近 50 条 */
export interface NoticeUnread { total: number; list: Notice[] }
