import { describe, expect, it } from 'vitest';
import type { ImportReport } from '../../../api/types';
import { summarize } from './report';

const report = (o: Partial<ImportReport>): ImportReport => ({
  kind: 'admin', total: 0, created: 0, rows: [], ...o,
});

describe('导入报告摘要', () => {
  it('全对 = success，坏行数 0', () => {
    const s = summarize(report({ total: 2, created: 2, rows: [{ line: 2, ok: true, msg: '' }, { line: 3, ok: true, msg: '' }] }));
    expect(s).toEqual({ level: 'success', bad: 0 });
  });

  it('有坏行 = error，并数出坏了几行（哪怕后端整批没写）', () => {
    const s = summarize(report({
      total: 2, created: 0,
      rows: [{ line: 2, ok: true, msg: '' }, { line: 3, ok: false, msg: '密码至少 8 位' }],
    }));
    expect(s).toEqual({ level: 'error', bad: 1 });
  });

  it('空文件（只有表头）= warn，不是错误', () => {
    expect(summarize(report({}))).toEqual({ level: 'warn', bad: 0 });
  });
});
