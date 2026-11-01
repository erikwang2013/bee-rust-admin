import { describe, expect, it } from 'vitest';
import { latestGuard } from './latest';

describe('latestGuard', () => {
  it('只认最后一次发起的请求：先发的后到也不认', () => {
    const g = latestGuard();
    const first = g.begin();
    const second = g.begin();
    expect(g.isCurrent(second)).toBe(true);
    expect(g.isCurrent(first)).toBe(false);
  });

  it('invalidate 作废在途请求（关弹窗 / 切新建 / 换篇）', () => {
    const g = latestGuard();
    const pending = g.begin();
    g.invalidate();
    expect(g.isCurrent(pending)).toBe(false);
  });

  it('作废后再发的新请求仍然有效', () => {
    const g = latestGuard();
    g.begin();
    g.invalidate();
    const next = g.begin();
    expect(g.isCurrent(next)).toBe(true);
  });
});
