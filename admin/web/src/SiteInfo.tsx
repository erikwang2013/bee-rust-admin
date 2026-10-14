import { theme } from 'antd';

/** 构建时注入（vite define，见 vite.config.ts）：发布包 = git tag，本地 / CI 构建 = dev。 */
declare const __APP_VERSION__: string;

const REPO_URL = 'https://github.com/erikwang2013/bee-rust-admin';

/**
 * 站点信息：版本 · 版权 · 项目地址，登录页底部与侧边栏底部共用，整行链到项目地址。
 * compact（侧边栏，220px 窄栏）：只是配色适配深色底，地址文字照显（放不下就折行）。
 */
export default function SiteInfo({ compact = false }: { compact?: boolean }) {
  const { token } = theme.useToken();
  return (
    <div
      style={{
        marginTop: 'auto', // 侧边栏包了 flex 列（见 BasicLayout）：把它压到最底；登录页非 flex，无影响
        padding: '12px 8px',
        fontSize: 12,
        lineHeight: 1.8,
        textAlign: 'center',
        overflowWrap: 'anywhere', // 窄栏里让长 URL 折行而不是撑破
        color: compact ? 'rgba(255,255,255,0.45)' : token.colorTextTertiary,
      }}
    >
      <a
        href={REPO_URL}
        target="_blank"
        rel="noreferrer"
        style={{ color: 'inherit', textDecoration: 'underline' }}
      >
        BRA {__APP_VERSION__} · © {new Date().getFullYear()} erik
        {` · ${REPO_URL.replace(/^https?:\/\//, '')}`}
      </a>
    </div>
  );
}
