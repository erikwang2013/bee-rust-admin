import { describe, expect, it } from 'vitest';
import { sizeText, visibleKind } from './format';

describe('sizeText', () => {
  it('按 1024 进制换单位，只有非整字节才带小数', () => {
    expect(sizeText(512)).toBe('512 B');
    expect(sizeText(2048)).toBe('2.0 KB');
    expect(sizeText(1536)).toBe('1.5 KB');
    expect(sizeText(5 * 1024 * 1024)).toBe('5.0 MB');
  });

  it('1023 还是字节、跨过 1024 才进位', () => {
    expect(sizeText(1023)).toBe('1023 B');
    expect(sizeText(1024)).toBe('1.0 KB');
  });

  it('没跑完（0）与脏值显示占位符，不冒 NaN', () => {
    expect(sizeText(0)).toBe('-');
    expect(sizeText(-1)).toBe('-');
    expect(sizeText(Number.NaN)).toBe('-');
  });
});

describe('visibleKind（发起类型的选择，修复轮 1）', () => {
  const both = [{ value: 'audit-log' }, { value: 'login-log' }];
  const onlyLogin = [{ value: 'login-log' }];

  it('选中项有查看码时原样返回', () => {
    expect(visibleKind(both, 'login-log')?.value).toBe('login-log');
    expect(visibleKind(both, 'audit-log')?.value).toBe('audit-log');
  });

  it('只有登录记录权限（默认选中 audit-log）回落第一个可见类型，不是没得导', () => {
    expect(visibleKind(onlyLogin, 'audit-log')?.value).toBe('login-log');
  });

  it('一个可见类型都没有才 undefined（页面显示「没有可导出的类型」）', () => {
    expect(visibleKind([], 'audit-log')).toBeUndefined();
  });
});
