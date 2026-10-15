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
