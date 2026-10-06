import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { downloadCsv } from './download';

// 这里只测 downloadCsv 自己的判断，axios/antd 那一层整体替掉
const { blob } = vi.hoisted(() => ({ blob: vi.fn() }));
vi.mock('./client', () => ({ http: { blob } }));

// Node 里没有 document / createObjectURL，用最小的桩替换掉，断言“点了哪个链接、下载了什么”
const anchor = { href: '', download: '', click: vi.fn() };
const createObjectURL = vi.fn(() => 'blob:stub');
const revokeObjectURL = vi.fn();

beforeEach(() => {
  blob.mockReset();
  anchor.href = '';
  anchor.download = '';
  anchor.click.mockClear();
  createObjectURL.mockClear();
  revokeObjectURL.mockClear();
  vi.stubGlobal('document', { createElement: () => anchor });
  vi.stubGlobal('URL', { createObjectURL, revokeObjectURL });
});

afterEach(() => vi.unstubAllGlobals());

describe('downloadCsv', () => {
  it('HTTP 200 但体是 JSON 信封时不落盘，抛后端消息', async () => {
    blob.mockResolvedValue({
      data: new Blob([JSON.stringify({ code: 40301, msg: '没有导出权限', data: null })], {
        type: 'application/json',
      }),
    });

    await expect(downloadCsv('/export/admins', undefined, 'admins')).rejects.toThrow('没有导出权限');
    expect(createObjectURL).not.toHaveBeenCalled();
    expect(anchor.click).not.toHaveBeenCalled();
  });

  it('JSON 信封没有 msg 时用默认文案', async () => {
    blob.mockResolvedValue({ data: new Blob(['{"code":500}'], { type: 'application/json' }) });

    await expect(downloadCsv('/export/admins', undefined, 'admins')).rejects.toThrow('导出失败');
    expect(anchor.click).not.toHaveBeenCalled();
  });

  it('正常 CSV：带上过滤参数取 blob，用 object URL 触发下载，用完即 revoke', async () => {
    const csv = new Blob(['id,name\n1,张三\n'], { type: 'text/csv' });
    blob.mockResolvedValue({ data: csv });

    await downloadCsv('/export/admins', { keyword: 'zhang' }, 'admins');

    expect(blob).toHaveBeenCalledWith('/export/admins', { keyword: 'zhang' });
    expect(createObjectURL).toHaveBeenCalledWith(csv);
    expect(anchor.href).toBe('blob:stub');
    expect(anchor.download).toMatch(/^admins-\d{8}\.csv$/);
    expect(anchor.click).toHaveBeenCalledTimes(1);
    expect(revokeObjectURL).toHaveBeenCalledWith('blob:stub');
  });
});
