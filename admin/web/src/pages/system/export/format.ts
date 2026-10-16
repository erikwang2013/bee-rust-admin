/** 字节 → 人能读的大小（导出文件从几 KB 的表头空表到几百 MB 都有）。 */
export function sizeText(n: number): string {
  // 0 / 负数 / NaN（没跑完的任务就是 0）都当「还没有」显示
  if (!Number.isFinite(n) || n <= 0) return '-';
  const units = ['B', 'KB', 'MB', 'GB'];
  let v = n;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i += 1;
  }
  return `${i === 0 ? v : v.toFixed(1)} ${units[i]}`;
}

/** 导出类型的选择：选中项不在可见列表里（或还没选）就回落第一个可见的。
 * 只有登录记录权限的账号默认项是 audit-log —— 不回落到这里，整块发起区会被
 * 「没有可导出的类型」顶掉，有权限也导不出（修复轮 1：U2 引入的回归）。 */
export function visibleKind<T extends { value: string }>(allowed: T[], kind: string): T | undefined {
  return allowed.find((k) => k.value === kind) ?? allowed[0];
}
