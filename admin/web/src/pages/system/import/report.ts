import type { ImportReport } from '../../../api/types';

/**
 * 报告摘要（纯函数，好单测）。后端口径是「有坏行就整批不写」（`created` 归 0），
 * 所以不能只看 `created`：空文件（只有表头）也是 0，但那不是错误 —— 三种结果分开说。
 */
export function summarize(r: ImportReport): { level: 'success' | 'warn' | 'error'; bad: number } {
  const bad = r.rows.filter((x) => !x.ok).length;
  if (bad > 0) return { level: 'error', bad };
  if (r.created === 0) return { level: 'warn', bad: 0 };
  return { level: 'success', bad: 0 };
}
