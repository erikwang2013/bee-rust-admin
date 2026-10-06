export interface ApiResult<T> { code: number; msg: string; data: T }
export interface Page<T> { list: T[]; total: number }

export interface UserInfo { id: number; username: string; nickname: string; avatar: string; is_super: boolean; dept_id: number }
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
export const DATA_SCOPE_LABELS: Record<number, string> = {
  1: '全部数据', 2: '本部门及以下', 3: '本部门', 4: '仅本人', 5: '自定义',
};
