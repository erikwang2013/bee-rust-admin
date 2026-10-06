import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from 'react';
import { Switch } from 'antd';
import { MoonOutlined, SunOutlined } from '@ant-design/icons';

export type ThemeMode = 'light' | 'dark';

const THEME_KEY = 'bee_admin_theme';

const ThemeCtx = createContext<{ mode: ThemeMode; toggle: () => void } | null>(null);

/** 记住的偏好优先，否则跟随系统。 */
function initialMode(): ThemeMode {
  const saved = localStorage.getItem(THEME_KEY);
  if (saved === 'light' || saved === 'dark') return saved;
  return window.matchMedia?.('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
}

export function ThemeProvider({ children }: { children: ReactNode }) {
  const [mode, setMode] = useState<ThemeMode>(initialMode);

  useEffect(() => { localStorage.setItem(THEME_KEY, mode); }, [mode]);

  const toggle = useCallback(() => setMode((m) => (m === 'dark' ? 'light' : 'dark')), []);
  const value = useMemo(() => ({ mode, toggle }), [mode, toggle]);

  return <ThemeCtx.Provider value={value}>{children}</ThemeCtx.Provider>;
}

export function useThemeMode() {
  const ctx = useContext(ThemeCtx);
  if (!ctx) throw new Error('useThemeMode 必须在 ThemeProvider 内使用');
  return ctx;
}

/** 顶栏与登录页共用的明暗开关。 */
export function ThemeToggle() {
  const { mode, toggle } = useThemeMode();
  return (
    <Switch
      aria-label="切换明暗主题"
      checked={mode === 'dark'}
      onChange={toggle}
      checkedChildren={<MoonOutlined />}
      unCheckedChildren={<SunOutlined />}
    />
  );
}
