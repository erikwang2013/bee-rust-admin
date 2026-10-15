import { useEffect, useState } from 'react';
import { theme } from 'antd';
import { fetchPublicConfig, type PublicConfig } from './api/config';

/** 构建时注入（vite define，见 vite.config.ts）：发布包 = git tag，本地 / CI 构建 = dev。 */
declare const __APP_VERSION__: string;

/** 接口挂了 / 键被删时的内置兜底（站点信息不能空着）。 */
const FALLBACK_REPO = 'https://github.com/erikwang2013/bee-rust-admin';

/** 取公开配置里的字符串值：非字符串或空白视同没配（走兜底）。 */
function str(v: string | boolean | undefined): string | null {
  return typeof v === 'string' && v.trim() !== '' ? v : null;
}

/**
 * 站点信息：版本 · 版权 · 项目地址，登录页底部与侧边栏底部共用，整行链到项目地址。
 * 版权文字与项目地址读 `/configs/public` 的 `site.copyright` / `site.repo_url`（改配置刷新即变），
 * 读不到就回落内置值；`site.footer_enabled = false` 时整块不渲染。
 * compact（侧边栏，220px 窄栏）：只是配色适配深色底，地址文字照显（放不下就折行）。
 */
export default function SiteInfo({ compact = false }: { compact?: boolean }) {
  const { token } = theme.useToken();
  const [cfg, setCfg] = useState<PublicConfig | null>(null);

  useEffect(() => {
    fetchPublicConfig()
      .then(setCfg)
      .catch(() => setCfg(null)); // 离线/接口挂了：保持 null，全部走内置值
  }, []);

  // 只有**显式**配置为 false 才隐藏；读不到（null/接口失败）不隐藏
  if (cfg?.['site.footer_enabled'] === false) return null;

  const repo = str(cfg?.['site.repo_url']) ?? FALLBACK_REPO;
  const copyright = str(cfg?.['site.copyright']) ?? `© ${new Date().getFullYear()} erik`;

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
        href={repo}
        target="_blank"
        rel="noreferrer"
        style={{ color: 'inherit', textDecoration: 'underline' }}
      >
        BRA {__APP_VERSION__} · {copyright}
        {` · ${repo.replace(/^https?:\/\//, '')}`}
      </a>
    </div>
  );
}
