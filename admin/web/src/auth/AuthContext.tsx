import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from 'react';
import { authApi } from '../api/auth';
import { TOKEN_KEY } from '../api/client';
import type { MenuNode, UserInfo } from '../api/types';

interface AuthState {
  user: UserInfo | null;
  perms: string[];
  menus: MenuNode[];
  ready: boolean;
  login: (username: string, password: string) => Promise<void>;
  logout: () => Promise<void>;
  reload: () => Promise<void>;
  has: (code: string) => boolean;
}

const AuthContext = createContext<AuthState | null>(null);

export function AuthProvider({ children }: { children: ReactNode }) {
  const [user, setUser] = useState<UserInfo | null>(null);
  const [perms, setPerms] = useState<string[]>([]);
  const [menus, setMenus] = useState<MenuNode[]>([]);
  const [ready, setReady] = useState(false);

  const reload = useCallback(async () => {
    if (!localStorage.getItem(TOKEN_KEY)) {
      setUser(null); setPerms([]); setMenus([]); setReady(true);
      return;
    }
    try {
      const profile = await authApi.profile();
      const tree = await authApi.menus();
      setUser(profile.user);
      setPerms(profile.perms);
      setMenus(tree);
    } catch {
      localStorage.removeItem(TOKEN_KEY);
      setUser(null);
    } finally {
      setReady(true);
    }
  }, []);

  useEffect(() => { void reload(); }, [reload]);

  const login = useCallback(async (username: string, password: string) => {
    const res = await authApi.login(username, password);
    localStorage.setItem(TOKEN_KEY, res.token);
    const profile = await authApi.profile();
    const tree = await authApi.menus();
    setUser(profile.user);
    setPerms(profile.perms);
    setMenus(tree);
    setReady(true);
  }, []);

  const logout = useCallback(async () => {
    try { await authApi.logout(); } catch { /* token 已失效也要能退出 */ }
    localStorage.removeItem(TOKEN_KEY);
    setUser(null); setPerms([]); setMenus([]);
  }, []);

  const has = useCallback(
    (code: string) => perms.includes('*:*:*') || perms.includes(code),
    [perms],
  );

  const value = useMemo(
    () => ({ user, perms, menus, ready, login, logout, reload, has }),
    [user, perms, menus, ready, login, logout, reload, has],
  );

  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>;
}

export function useAuth(): AuthState {
  const ctx = useContext(AuthContext);
  if (!ctx) throw new Error('useAuth 必须在 AuthProvider 内使用');
  return ctx;
}
