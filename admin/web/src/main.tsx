import React from 'react';
import ReactDOM from 'react-dom/client';
import { ConfigProvider, App as AntdApp, theme as antdTheme } from 'antd';
import antdZhCN from 'antd/locale/zh_CN';
import antdEnUS from 'antd/locale/en_US';
import { RouterProvider } from 'react-router';
import { router } from './router';
import { AuthProvider } from './auth/AuthContext';
import { ThemeProvider, useThemeMode } from './theme';
import { I18nProvider, useI18n } from './i18n';
import { ErrorBoundary } from './ErrorBoundary';

function ThemedApp() {
  const { mode } = useThemeMode();
  const { lang } = useI18n();
  return (
    <ConfigProvider
      locale={lang === 'zh-CN' ? antdZhCN : antdEnUS}
      theme={{ algorithm: mode === 'dark' ? antdTheme.darkAlgorithm : antdTheme.defaultAlgorithm }}
    >
      <AntdApp>
        <AuthProvider>
          <RouterProvider router={router} />
        </AuthProvider>
      </AntdApp>
    </ConfigProvider>
  );
}

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    {/* 最外层：登录页也一并兜住，见 ErrorBoundary.tsx 的说明 */}
    <ErrorBoundary>
      <I18nProvider>
        <ThemeProvider>
          <ThemedApp />
        </ThemeProvider>
      </I18nProvider>
    </ErrorBoundary>
  </React.StrictMode>,
);
