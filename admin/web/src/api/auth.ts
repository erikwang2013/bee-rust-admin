import { http } from './client';
import type { MenuNode, Profile, UserInfo } from './types';

export interface LoginResult { token: string; expires_in: number; user: UserInfo }

export const authApi = {
  login: (username: string, password: string) =>
    http.post<LoginResult>('/auth/login', { username, password }),
  logout: () => http.post<null>('/auth/logout'),
  /** 退出其他设备：后端递增 token_version 踢掉旧 token，并给当前设备换发新 token */
  logoutOthers: () => http.post<Pick<LoginResult, 'token' | 'expires_in'>>('/auth/logout-others'),
  profile: () => http.get<Profile>('/auth/profile'),
  menus: () => http.get<MenuNode[]>('/auth/menus'),
  changePassword: (old_password: string, new_password: string) =>
    http.put<null>('/auth/password', { old_password, new_password }),
  updateProfile: (data: { nickname: string; email: string; phone: string }) =>
    http.put<null>('/auth/profile', data),
  uploadAvatar: (data_url: string) => http.post<{ avatar: string }>('/auth/avatar', { data_url }),
};
