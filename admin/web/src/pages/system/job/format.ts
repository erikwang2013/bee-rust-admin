import { t } from '../../../i18n';

/** 间隔秒（协议上是字符串）→ 人话：列表里 86400 这种数字看不出是「一天」。 */
export function intervalText(cron: string): string {
  const sec = Number(cron);
  // 解析不出正整数（空值、脏数据、将来可能换成的表达式）就原样回显，别瞎猜
  if (!Number.isInteger(sec) || sec <= 0) return cron.trim() || '-';
  if (sec % 86400 === 0) return t('job.day', { n: sec / 86400 });
  if (sec % 3600 === 0) return t('job.hour', { n: sec / 3600 });
  if (sec % 60 === 0) return t('job.min', { n: sec / 60 });
  return t('job.sec', { n: sec });
}
