import { beforeEach, describe, expect, it, vi } from 'vitest';
import { uploadAttachment } from './attach';
import type { AttachmentRow } from './types';

// 网络层整体替掉，只测附件这一层的差异（分片内核本身由 avatarUpload.test.ts 覆盖）
const { post } = vi.hoisted(() => ({ post: vi.fn() }));
vi.mock('./client', () => ({ http: { post } }));

const ROW: AttachmentRow = {
  id: 'abc12345', name: 'logo.png', ext: 'png', mime: 'image/png', size: 4, sha256: 'x',
  admin_id: 'adm00001', admin_name: 'admin', entity_type: '', entity_id: '', entity_label: '',
  is_public: false, sort: 0, created_at: '2026-10-16 10:00:00',
};

const preprocessOk = {
  chunk_size: 4,
  group_sub_dir: '202610',
  resource_temp_base_name: 'tmp123',
  resource_ext: 'png',
  saved_path: '',
};

const fileOf = (n: number, name = 'logo.png') =>
  new File([Uint8Array.from({ length: n }, (_, i) => i + 48)], name, { type: 'image/png' });

beforeEach(() => post.mockReset());

describe('uploadAttachment', () => {
  it('走 attachments 两个接口 + group=attach，并把原始文件名放进 name 字段', async () => {
    post.mockResolvedValueOnce(preprocessOk);
    post.mockResolvedValueOnce({ saved_path: '202610/abc.png', attachment: ROW });

    const row = await uploadAttachment(fileOf(4, 'logo.png'), { locale: 'zh' });

    expect(row.id).toBe(ROW.id);
    expect(post.mock.calls.map((c) => c[0]))
      .toEqual(['/attachments/upload/preprocess', '/attachments/upload/chunk']);
    const head = post.mock.calls[0][1] as FormData;
    expect(head.get('group')).toBe('attach');
    // 插件不认 name，后端拿它当原始文件名（漏了就只能用落盘名）
    expect(head.get('name')).toBe('logo.png');
  });

  it('秒传命中：一个分片都不发，回包里的 attachment 行直接生效', async () => {
    post.mockResolvedValueOnce({ ...preprocessOk, saved_path: '202610/exists.png', attachment: ROW });

    await expect(uploadAttachment(fileOf(4), { locale: 'zh' })).resolves.toEqual(ROW);
    expect(post).toHaveBeenCalledTimes(1);
  });

  it('末片没带回 attachment 行（服务端没按契约回）按失败处理', async () => {
    post.mockResolvedValueOnce(preprocessOk);
    post.mockResolvedValueOnce({ saved_path: '202610/abc.png' });

    await expect(uploadAttachment(fileOf(4), { locale: 'zh' })).rejects.toThrow('附件上传失败');
  });
});
