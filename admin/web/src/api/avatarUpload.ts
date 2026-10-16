import { t } from '../i18n';
import { UploadError, uploadChunked } from './chunkedUpload';

/** 头像上传失败（协议异常）。HTTP 层的失败已由拦截器提示，不走这个类。 */
export { UploadError as AvatarUploadError };

export interface AvatarUploadOptions {
  /** `zh` / `en`：插件的错误文案按它出 */
  locale: 'zh' | 'en';
  /** 进度：已确认送达的字节 / 总字节 */
  onProgress?: (sent: number, total: number) => void;
}

/** 传完整个头像（分片契约见 [chunkedUpload]），成功返回（头像 URL 形状不变，调用方 reload 资料即可刷新）。 */
export async function uploadAvatar(file: File, opts: AvatarUploadOptions): Promise<void> {
  await uploadChunked({
    file,
    group: 'avatar',
    locale: opts.locale,
    onProgress: opts.onProgress,
    preprocessPath: '/avatar/upload/preprocess',
    chunkPath: '/avatar/upload/chunk',
    failMsg: t('profile.avatar_upload_failed'),
  });
}
