/**
 * 站点信息公开配置的读取与兜底（纯逻辑，`siteinfo.test.ts` 覆盖；渲染见 `SiteInfo.tsx`）。
 * 抽出来是为了让「拿不到配置时的回落」与「地址 scheme 白名单」这两条安全相关的分支
 * 有单测兜底，不必拉起 React/antd。
 */

/** 接口挂了 / 键被删时的内置兜底（站点信息不能空着）。 */
export const FALLBACK_REPO = 'https://github.com/erikwang2013/bee-rust-admin';

/** 取公开配置里的字符串值：非字符串或空白视同没配（走兜底）。 */
export function str(v: string | boolean | undefined): string | null {
  return typeof v === 'string' && v.trim() !== '' ? v : null;
}

/**
 * 项目地址只认 http(s)：这个值直接进 `<a href>`，而链接渲染在**未登录**的登录页上 ——
 * `javascript:` / `data:` 之类的 scheme 是配置编辑者（system:config:edit 可单独授予）
 * 打访客与超管的存储型 XSS 面。非 http(s) 一律回落内置地址（仓库没有 CSP，不能靠它兜底）。
 */
export function safeRepoUrl(v: string | boolean | undefined): string {
  const s = str(v);
  return s && /^https?:\/\//i.test(s) ? s : FALLBACK_REPO;
}

/** 版权文字兜底：`© <当前年> erik`（配置里的值优先，空白走这里）。 */
export function copyrightText(v: string | boolean | undefined, now: Date = new Date()): string {
  return str(v) ?? `© ${now.getFullYear()} erik`;
}
