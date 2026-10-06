import type { JobRunResult } from '../../../api/types';

/** 间隔秒（协议上是字符串）→ 人话：列表里 86400 这种数字看不出是「一天」。 */
export function intervalText(cron: string): string {
  const sec = Number(cron);
  // 解析不出正整数（空值、脏数据、将来可能换成的表达式）就原样回显，别瞎猜
  if (!Number.isInteger(sec) || sec <= 0) return cron.trim() || '-';
  if (sec % 86400 === 0) return `${sec / 86400} 天`;
  if (sec % 3600 === 0) return `${sec / 3600} 小时`;
  if (sec % 60 === 0) return `${sec / 60} 分钟`;
  return `${sec} 秒`;
}

/**
 * 手动触发的提示文案：成功失败都要把后端 msg 原样给用户 ——
 * 它就是写进 job_log 的那条（如「清理 12 条」），比任何前端套话都有用。
 */
export function runResult(r: JobRunResult): { ok: boolean; text: string } {
  const base = r.msg || (r.status === 1 ? '执行完成' : '执行失败');
  return { ok: r.status === 1, text: `${base}（耗时 ${r.duration_ms} ms）` };
}
