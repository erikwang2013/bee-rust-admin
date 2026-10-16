import { t } from '../i18n';
import { http } from './client';
import { UploadError, uploadChunked } from './chunkedUpload';
import type { AttachmentRow, Page } from './types';

export interface AttachQuery {
  page?: number; size?: number;
  /** 原始文件名模糊 */
  name?: string;
  /** 关联类型：`notice` = 公告配图，`none` = 只看未绑定，省略 = 全部 */
  entity_type?: string;
  /** 只看某个对象的附件（与 entity_type 一起用，值是对外短串） */
  entity_id?: string;
}

export interface AttachBindBody {
  /** 关联类型；空串 = 解绑 */
  entity_type: string;
  entity_id: string;
  sort?: number;
}

export const attachApi = {
  list: (q: AttachQuery) => http.get<Page<AttachmentRow>>('/attachments', q),
  /** 关联 / 改排序（`entity_type` 传空串即解绑） */
  bind: (id: string, body: AttachBindBody) => http.put<AttachmentRow>(`/attachments/${id}`, body),
  remove: (id: string) => http.del<{ deleted: number }>(`/attachments/${id}`),
  /**
   * 下载走 blob（`window.open` 不带 Authorization 头）；文件名用原始文件名，
   * 不是 hashids 短串。没权限时后端 403，拦截器按 403 提示。
   */
  download: async (row: AttachmentRow) => {
    const res = await http.blob(`/attachments/${row.id}`);
    const href = URL.createObjectURL(res.data);
    const a = document.createElement('a');
    a.href = href;
    a.download = row.name;
    a.click();
    URL.revokeObjectURL(href);
  },
};

/** 上传附件（分片契约与头像同一套，见 [chunkedUpload]），返回落库后的行。 */
export async function uploadAttachment(
  file: File,
  opts: { locale: 'zh' | 'en'; onProgress?: (sent: number, total: number) => void },
): Promise<AttachmentRow> {
  const reply = await uploadChunked({
    file,
    group: 'attach',
    locale: opts.locale,
    onProgress: opts.onProgress,
    preprocessPath: '/attachments/upload/preprocess',
    chunkPath: '/attachments/upload/chunk',
    failMsg: t('attach.upload_failed'),
    // 插件不认 name，后端拿它当原始文件名；漏了就只能用落盘名（时间戳 + 摘要）
    extra: { name: file.name },
  });
  const row = reply.attachment as AttachmentRow | undefined;
  // 秒传命中与末片都会带 attachment 行；没带 = 服务端没按契约回
  if (!row) throw new UploadError(t('attach.upload_failed'));
  return row;
}
