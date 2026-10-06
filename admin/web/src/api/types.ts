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
  id: number; username: string; nickname: string; avatar: string; is_super: boolean; dept_id: number;
  /** 后端 /auth/profile 返回时回填资料表单；未返回则为空 */
  email?: string; phone?: string;
}
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
  /**
   * 当前操作者能不能把这个角色授出去（后端角色列表/详情带的权威标记）。
   * 前端只认这个布尔值，不自己算 data_scope / 权限码；保存时后端还会硬校验一次。
   */
  grantable?: boolean;
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
export interface AuditLog {
  id: number; admin_id: number; username: string; module: string; action: string;
  method: string; path: string; status: number; msg: string;
  duration_ms: number; ip: string; created_at: string;
}
export interface DictType {
  id: number; name: string; code: string; status: number; remark: string; created_at: string;
}
export interface DictItem {
  id: number; type_code: string; label: string; value: string; sort: number;
  status: number; remark: string; created_at: string;
}
export interface Job {
  id: number; name: string; code: string;
  /** 间隔秒数的**字符串**（列是 varchar64）：列名保留 cron 是为了将来换表达式时不改列名 */
  cron: string; status: number;
  /** 从未跑过时为空 */
  last_run_at: string | null; last_status: number | null; last_msg: string;
  created_at: string; updated_at: string;
}
export interface JobLog {
  id: number; job_code: string; started_at: string; duration_ms: number;
  status: number; msg: string;
}
/** 手动触发（同步执行）的结果：status 1 成功 / 0 失败，msg 就是写进 job_log 的那条 */
export interface JobRunResult { status: number; msg: string; duration_ms: number }

export interface Notice {
  id: number; title: string; content: string;
  /** 0 草稿 / 1 已发布。取消发布不清 published_at（留痕：曾发布过） */
  status: number; created_by: number; published_at: string | null;
  created_at: string; updated_at: string;
}
/** 未读接口：total 是未读总数（铃铛角标用它），list 只给最近 50 条 */
export interface NoticeUnread { total: number; list: Notice[] }
