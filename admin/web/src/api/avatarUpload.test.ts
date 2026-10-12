import { beforeEach, describe, expect, it, vi } from 'vitest';
import { AvatarUploadError, uploadAvatar } from './avatarUpload';
import { md5Hex } from './md5';

// 网络层整体替掉，只测分片逻辑本身（和 download.test.ts 一个路子）
const { post } = vi.hoisted(() => ({ post: vi.fn() }));
vi.mock('./client', () => ({ http: { post } }));

const PREPROCESS = '/avatar/upload/preprocess';
const CHUNK = '/avatar/upload/chunk';

// 契约：aetherupload-rust 2.x 的成功体是 snake_case（1.x 的 camelCase + `error` 字段已废除）
const preprocessOk = {
  chunk_size: 4,
  group_sub_dir: '202610',
  resource_temp_base_name: 'tmp123',
  resource_ext: 'jpg',
  saved_path: '',
};

/** 10 字节 → 4+4+2 三片（末片不足一片）。 */
const bytes = (n: number) => Uint8Array.from({ length: n }, (_, i) => i + 48);
const fileOf = (n: number, name = 'avatar.jpg') => new File([bytes(n)], name, { type: 'image/jpeg' });

const formsOf = (from: number) => post.mock.calls.slice(from).map((c) => c[1] as FormData);

beforeEach(() => post.mockReset());

describe('uploadAvatar 分片上传', () => {
  it('按服务端 chunk_size 切片：索引从 1 开始，末片不足一片，末片 saved_path 非空即完成', async () => {
    post.mockResolvedValueOnce(preprocessOk);
    post.mockResolvedValueOnce({ saved_path: '' });
    post.mockResolvedValueOnce({ saved_path: '' });
    post.mockResolvedValueOnce({ saved_path: '202610/abc.jpg' });

    const progress: [number, number][] = [];
    await uploadAvatar(fileOf(10), { locale: 'zh', onProgress: (s, t) => progress.push([s, t]) });

    expect(post.mock.calls.map((c) => c[0])).toEqual([PREPROCESS, CHUNK, CHUNK, CHUNK]);
    const chunks = formsOf(1);
    expect(chunks.map((f) => f.get('chunk_index'))).toEqual(['1', '2', '3']);
    expect(chunks.map((f) => f.get('chunk_total'))).toEqual(['3', '3', '3']);
    expect(chunks.map((f) => (f.get('resource_chunk') as File).size)).toEqual([4, 4, 2]);
    // 进度是「已确认送达的字节」
    expect(progress).toEqual([[4, 10], [8, 10], [10, 10]]);
  });

  it('preprocess 带上原文件名、体积、md5 与 locale；chunk 带上插件要的上下文字段', async () => {
    post.mockResolvedValueOnce(preprocessOk);
    post.mockResolvedValueOnce({ saved_path: '202610/abc.jpg' });

    await uploadAvatar(fileOf(4, 'my-photo.jpg'), { locale: 'en' });

    const head = post.mock.calls[0][1] as FormData;
    expect(head.get('resource_name')).toBe('my-photo.jpg');
    expect(head.get('resource_size')).toBe('4');
    expect(head.get('resource_hash')).toBe(md5Hex(bytes(4)));
    expect(head.get('group')).toBe('avatar');
    expect(head.get('locale')).toBe('en');

    const chunk = post.mock.calls[1][1] as FormData;
    expect(chunk.get('resource_hash')).toBe(md5Hex(bytes(4)));
    expect(chunk.get('resource_temp_basename')).toBe('tmp123');
    expect(chunk.get('resource_ext')).toBe('jpg');
    expect(chunk.get('group_subdir')).toBe('202610');
    expect(chunk.get('group')).toBe('avatar');
    expect(chunk.get('locale')).toBe('en');
  });

  it('刚好整除时末片是整片（不会多切一片空的）', async () => {
    post.mockResolvedValueOnce(preprocessOk);
    post.mockResolvedValueOnce({ saved_path: '' });
    post.mockResolvedValueOnce({ saved_path: '202610/abc.jpg' });

    await uploadAvatar(fileOf(8), { locale: 'zh' });

    const chunks = formsOf(1);
    expect(chunks).toHaveLength(2);
    expect(chunks.map((f) => (f.get('resource_chunk') as File).size)).toEqual([4, 4]);
  });

  it('秒传命中：一个分片都不发，直接完成', async () => {
    post.mockResolvedValueOnce({ ...preprocessOk, saved_path: '202610/exists.jpg' });

    const progress: [number, number][] = [];
    await uploadAvatar(fileOf(10), { locale: 'zh', onProgress: (s, t) => progress.push([s, t]) });

    expect(post).toHaveBeenCalledTimes(1);
    expect(progress).toEqual([[10, 10]]);
  });

  it('中途一片 HTTP 失败：原样抛出，不静默吞掉', async () => {
    post.mockResolvedValueOnce(preprocessOk);
    post.mockRejectedValueOnce(new Error('Network Error'));

    await expect(uploadAvatar(fileOf(10), { locale: 'zh' })).rejects.toThrow('Network Error');
    expect(post).toHaveBeenCalledTimes(2);
  });

  it('分片全传完却没等到 saved_path：按失败处理', async () => {
    post.mockResolvedValueOnce(preprocessOk);
    post.mockResolvedValueOnce({ saved_path: '' });

    await expect(uploadAvatar(fileOf(4), { locale: 'zh' })).rejects.toThrow('头像上传失败');
  });

  it('服务端没给合法 chunk_size 时不发分片，报兜底文案', async () => {
    post.mockResolvedValueOnce({ ...preprocessOk, chunk_size: 0 });

    await expect(uploadAvatar(fileOf(10), { locale: 'zh' })).rejects.toThrow('头像上传失败');
    expect(post).toHaveBeenCalledTimes(1);
  });

  it('失败经 HTTP 层抛出（2.x 的失败体不再混在成功回包里）', async () => {
    // 2.x：插件失败 → 后端转成 400 信封（msg = Problem.title），由 axios 拦截器统一抛
    post.mockRejectedValueOnce(new AvatarUploadError('错误：上传数据超过大小限制'));

    await expect(uploadAvatar(fileOf(10), { locale: 'zh' }))
      .rejects.toThrow('错误：上传数据超过大小限制');
    expect(post).toHaveBeenCalledTimes(1);
  });
});
