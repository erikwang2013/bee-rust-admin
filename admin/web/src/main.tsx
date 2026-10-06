import React from 'react';
import ReactDOM from 'react-dom/client';
import { ConfigProvider, App as AntdApp, theme as antdTheme } from 'antd';
// 走 es/ 而不是 'antd/locale/xx_XX'：后者是 CJS 包装（`module.exports = require('../lib/…)`），
// 会把 lib 与 es 两份语言包都打进包里。es 版是同一份数据的 ESM 版，只留一份。
import antdZhCN from 'antd/es/locale/zh_CN';
import antdEnUS from 'antd/es/locale/en_US';
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
