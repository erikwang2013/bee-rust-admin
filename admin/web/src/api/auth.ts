import { http } from './client';
import type { MenuNode, Profile, UserInfo } from './types';

export interface LoginResult { token: string; expires_in: number; user: UserInfo }

export const authApi = {
  login: (username: string, password: string) =>
    http.post<LoginResult>('/auth/login', { username, password }),
  logout: () => http.post<null>('/auth/logout'),
  profile: () => http.get<Profile>('/auth/profile'),
  menus: () => http.get<MenuNode[]>('/auth/menus'),
  changePassword: (old_password: string, new_password: string) =>
    http.put<null>('/auth/password', { old_password, new_password }),
};
