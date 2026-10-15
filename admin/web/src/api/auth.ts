import { http } from './client';
import type { LoginDevice, MenuNode, Profile, TotpSetup, UserInfo } from './types';

export interface LoginResult { token: string; expires_in: number; user: UserInfo }

export const authApi = {
  logout: () => http.post<null>('/auth/logout'),
  /** 退出其他设备：后端递增 token_version 踢掉旧 token，并给当前设备换发新 token */
  logoutOthers: () => http.post<Pick<LoginResult, 'token' | 'expires_in'>>('/auth/logout-others'),
  profile: () => http.get<Profile>('/auth/profile'),
  menus: () => http.get<MenuNode[]>('/auth/menus'),
  changePassword: (old_password: string, new_password: string) =>
    http.put<null>('/auth/password', { old_password, new_password }),
  updateProfile: (data: { nickname: string; email: string; phone: string }) =>
    http.put<null>('/auth/profile', data),
};

/**
 * 动态验证码与登录设备（项 7）。全是自服务接口（管自己的东西），不挂权限码；
 * 密钥只在 setup 那一次回明文 —— 别把它存进任何长期状态里。
 */
export const totpApi = {
  setup: () => http.post<TotpSetup>('/auth/totp/setup'),
  enable: (code: string) => http.post<null>('/auth/totp/enable', { code }),
  disable: (code: string) => http.post<null>('/auth/totp/disable', { code }),
  devices: () => http.get<LoginDevice[]>('/auth/devices'),
  /** 踢一台设备下线：它手里的 token 下一次请求就 401 */
  kick: (id: string) => http.del<null>(`/auth/devices/${id}`),
};
