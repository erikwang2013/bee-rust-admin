import { describe, expect, it } from 'vitest';
import { sizeText } from './format';

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
