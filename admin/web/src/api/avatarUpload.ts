import { t } from '../i18n';
import { http } from './client';
import { md5Hex } from './md5';

/**
 * 头像分片上传（aetherupload-rust 2.x 契约，见
 * docs/superpowers/plans/2026-10-06-bra-v1.5-c-modules.md「阶段 4b 契约」及其补记）。
 * 两个接口都是 multipart 表单，`data` 是插件原文（`chunk_size` / `saved_path` …）——
 * **成功体里没有 1.x 那个 `error` 字段**；失败是插件的 Problem，后端按它的真实状态码
 * （400 / 409 / 413 / 415 …）与稳定码转成统一信封（`msg` 是插件文案）。HTTP 层的失败由
 * 拦截器统一提示，不走下面的 AvatarUploadError。
 * 没有 md5 时插件（非宽松模式）末片校验会直接把整份上传判失败，所以这里带着真 hash 传。
 */
interface PreprocessData {
  /** 服务端定的分片字节数，前端必须用它（不要自己写死） */
  chunk_size: number;
  group_sub_dir: string;
  resource_temp_base_name: string;
  resource_ext: string;
  /** 非空 = 秒传命中，一个分片都不用传 */
  saved_path: string;
}

interface ChunkData {
  /** 非空 = 最后一片组装完成 */
  saved_path: string;
}

/** 需要显示到界面的失败（插件文案 / 兜底文案）。HTTP 层的失败已由拦截器提示，不走这个类。 */
export class AvatarUploadError extends Error {}

export interface AvatarUploadOptions {
  /** `zh` / `en`：插件的错误文案按它出 */
  locale: 'zh' | 'en';
  /** 进度：已确认送达的字节 / 总字节 */
  onProgress?: (sent: number, total: number) => void;
}

const GROUP = 'avatar';

/** 传完整个头像，成功返回（头像 URL 形状不变，调用方 reload 资料即可刷新）。 */
export async function uploadAvatar(file: File, opts: AvatarUploadOptions): Promise<void> {
  const { locale, onProgress } = opts;
  const total = file.size;
  const hash = md5Hex(new Uint8Array(await file.arrayBuffer()));

  const pre = new FormData();
  pre.append('resource_name', file.name);
  pre.append('resource_size', String(total));
  pre.append('resource_hash', hash);
  pre.append('group', GROUP);
  pre.append('locale', locale);

  const head = await http.post<PreprocessData>('/avatar/upload/preprocess', pre);
  if (head.saved_path) {
    // 秒传命中（同 hash 传过）：直接完成
    onProgress?.(total, total);
    return;
  }

  const chunkSize = head.chunk_size;
  if (!(chunkSize > 0)) throw new AvatarUploadError(t('profile.avatar_upload_failed'));
  const chunks = Math.ceil(total / chunkSize);
  let sent = 0;
  for (let index = 1; index <= chunks; index++) {
    const start = (index - 1) * chunkSize;
    const slice = file.slice(start, Math.min(total, start + chunkSize));
    const form = new FormData();
    form.append('resource_chunk', slice, file.name);
    form.append('chunk_total', String(chunks));
    form.append('chunk_index', String(index)); // 从 1 开始
    form.append('resource_temp_basename', head.resource_temp_base_name);
    form.append('resource_ext', head.resource_ext);
    form.append('group_subdir', head.group_sub_dir);
    form.append('resource_hash', hash);
    form.append('group', GROUP);
    form.append('locale', locale);

    const reply = await http.post<ChunkData>('/avatar/upload/chunk', form);
    sent += slice.size;
    onProgress?.(sent, total);
    if (reply.saved_path) return; // 末片组装完成
  }
  // 分片全传完却没拿到 savedPath：服务端没按契约回，不能当成功
  throw new AvatarUploadError(t('profile.avatar_upload_failed'));
}
