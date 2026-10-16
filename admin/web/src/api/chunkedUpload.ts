import { http } from './client';
import { md5Hex } from './md5';

/**
 * 分片上传内核（aetherupload-rust 3.x 契约，见
 * docs/superpowers/plans/2026-10-06-bra-v1.5-c-modules.md「阶段 4b 契约」及其补记）。
 * 头像与附件共用这一份：两者只有分组名和接口路径不同，切片/进度/失败语义完全一样。
 * 两个接口都是 multipart 表单，`data` 是插件原文（`chunk_size` / `saved_path` …）——
 * **成功体里没有 1.x 那个 `error` 字段**；失败是插件的 Problem，后端按它的真实状态码
 * （400 / 409 / 413 / 415 …）与稳定码转成统一信封（`msg` 是插件文案）。HTTP 层的失败由
 * 拦截器统一提示，不走下面的 UploadError。
 * 没有 md5 时插件（非宽松模式）末片校验会直接把整份上传判失败，所以这里带着真 hash 传。
 */
export interface UploadReply {
  /** 服务端定的分片字节数，前端必须用它（不要自己写死） */
  chunk_size?: number;
  group_sub_dir?: string;
  resource_temp_base_name?: string;
  resource_ext?: string;
  /** 非空 = 秒传命中 / 末片组装完成，一个分片都不用传（或本次已传完） */
  saved_path?: string;
  /** 后端在插件回包上额外塞的字段（附件上传会带 `attachment` 行） */
  [key: string]: unknown;
}

/** 需要显示到界面的失败（协议异常）。HTTP 层的失败已由拦截器提示，不走这个类。 */
export class UploadError extends Error {}

export interface ChunkedUploadOptions {
  file: File;
  /** 插件分组名：`avatar` / `attach`（后端按它挑白名单与落盘目录） */
  group: string;
  /** `zh` / `en`：插件的错误文案按它出 */
  locale: 'zh' | 'en';
  preprocessPath: string;
  chunkPath: string;
  /** 协议异常（没给合法 chunk_size / 分片传完仍无 saved_path）时的兜底文案 */
  failMsg: string;
  /** preprocess 的额外字段（附件要带 `name` = 原始文件名） */
  extra?: Record<string, string>;
  /** 进度：已确认送达的字节 / 总字节 */
  onProgress?: (sent: number, total: number) => void;
}

/** 传完整个文件，返回最后一次回包（末片或秒传命中的预检回包）。 */
export async function uploadChunked(o: ChunkedUploadOptions): Promise<UploadReply> {
  const { file, group, locale, preprocessPath, chunkPath, failMsg, extra, onProgress } = o;
  const total = file.size;
  const hash = md5Hex(new Uint8Array(await file.arrayBuffer()));

  const pre = new FormData();
  pre.append('resource_name', file.name);
  pre.append('resource_size', String(total));
  pre.append('resource_hash', hash);
  pre.append('group', group);
  pre.append('locale', locale);
  for (const [k, v] of Object.entries(extra ?? {})) pre.append(k, v);

  const head = await http.post<UploadReply>(preprocessPath, pre);
  if (head.saved_path) {
    // 秒传命中（同 hash 传过）：直接完成
    onProgress?.(total, total);
    return head;
  }

  const chunkSize = head.chunk_size ?? 0;
  if (!(chunkSize > 0)) throw new UploadError(failMsg);
  const chunks = Math.ceil(total / chunkSize);
  let sent = 0;
  for (let index = 1; index <= chunks; index++) {
    const start = (index - 1) * chunkSize;
    const slice = file.slice(start, Math.min(total, start + chunkSize));
    const form = new FormData();
    form.append('resource_chunk', slice, file.name);
    form.append('chunk_total', String(chunks));
    form.append('chunk_index', String(index)); // 从 1 开始
    form.append('resource_temp_basename', head.resource_temp_base_name ?? '');
    form.append('resource_ext', head.resource_ext ?? '');
    form.append('group_subdir', head.group_sub_dir ?? '');
    form.append('resource_hash', hash);
    form.append('group', group);
    form.append('locale', locale);

    const reply = await http.post<UploadReply>(chunkPath, form);
    sent += slice.size;
    onProgress?.(sent, total);
    if (reply.saved_path) return reply; // 末片组装完成
  }
  // 分片全传完却没拿到 savedPath：服务端没按契约回，不能当成功
  throw new UploadError(failMsg);
}
