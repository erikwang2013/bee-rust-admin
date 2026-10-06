import { Component, type ErrorInfo, type ReactNode } from 'react';
import { Button, Result } from 'antd';
import { useRouteError } from 'react-router';
import { t } from './i18n';

/**
 * 出错时的降级界面：可读的说明 + 阿守 + 刷新按钮，取代白屏。
 *
 * 刻意做得很"素"——不读主题/ConfigProvider 上下文，因为它可能正是出错的那一环；
 * 只用 antd 默认样式 + 内联样式。传进来的 error 可能是任何类型（路由的
 * errorElement 里拿到的不保证是 Error），所以统一在这里收敛。
 *
 * 文案用模块级的 `t`，**不能**用 useI18n()：最外层这个 ErrorBoundary 就在
 * I18nProvider 外面，出错时正好可能连 Provider 都没渲染出来。
 */
function CrashScreen({ error }: { error: unknown }) {
  const msg = error instanceof Error ? error.message : typeof error === 'string' ? error : '';
  return (
    <Result
      style={{ paddingTop: 96 }}
      icon={<img src="/keeper-alarmed.svg" alt="" width={96} height={96} />}
      title={t('error.crash_title')}
      subTitle={msg || t('error.crash_unknown')}
      // 出错后整棵树已经不可信，重新挂载比局部恢复更可靠
      extra={<Button type="primary" onClick={() => window.location.reload()}>{t('error.crash_retry')}</Button>}
    />
  );
}

/**
 * 路由层的兜底（router.tsx 里挂成根路由的 errorElement）。
 *
 * 这是真正干活的那个：react-router 给每条路由都套了自己的错误边界，
 * 路由内容渲染时抛的错（页面分包下载失败、图标懒加载分包下载失败、
 * 任何组件渲染抛错）都会被它先接住，**根本传不到外层的 ErrorBoundary**。
 * 实测过：不挂这个，界面会落到 react-router 那个英文通用错误页。
 */
export function RouteError() {
  return <CrashScreen error={useRouteError()} />;
}

/**
 * 最外层的兜底（main.tsx 里包住 ThemeProvider/ConfigProvider/AuthProvider）。
 *
 * 路由之外的东西出错时归它管 —— 主要是几个 Provider 自己的渲染异常，
 * 以及未来任何挂在 RouterProvider 之外的内容。只留路由那层的话，
 * AuthProvider 渲染时抛错照样白屏。
 */
export class ErrorBoundary extends Component<{ children: ReactNode }, { error: Error | null }> {
  state: { error: Error | null } = { error: null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error('[ErrorBoundary]', error, info.componentStack);
  }

  render() {
    return this.state.error ? <CrashScreen error={this.state.error} /> : this.props.children;
  }
}
