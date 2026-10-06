import dayjs from 'dayjs';
import { t } from '../i18n';
import { http } from './client';

/**
 * 带 Bearer 的附件下载：window.open 不会带 Authorization 头，所以走 axios 取 blob，
 * 再用临时 <a download> 落盘。文件名形如 admins-20261006.csv。
 */
export async function downloadCsv(url: string, params: object | undefined, name: string) {
  const res = await http.blob(url, params);
  const blob = res.data;

  // 后端出错时可能仍以 200 回一个 JSON 信封，直接落盘会得到一份假 CSV
  if (blob.type.includes('json')) {
    const body = JSON.parse(await blob.text()) as { msg?: string };
    throw new Error(body.msg || t('common.export_failed'));
  }

  const href = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = href;
  a.download = `${name}-${dayjs().format('YYYYMMDD')}.csv`;
  a.click();
  URL.revokeObjectURL(href);
}
