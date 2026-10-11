import { describe, expect, it } from 'vitest';
import { isStrongPassword } from './password';

describe('isStrongPassword', () => {
  it('短于 8 字符不通过（哪怕类别够）', () => {
    expect(isStrongPassword('short7x')).toBe(false); // 7 位，小写+数字
  });

  it('够 8 位但只有一类不通过', () => {
    expect(isStrongPassword('aaaaaaaa')).toBe(false); // 单小写
    expect(isStrongPassword('!@#$%^&*')).toBe(false); // 单符号
  });

  it('够 8 位且两类通过', () => {
    expect(isStrongPassword('aaaaaaaa1')).toBe(true); // 小写 + 数字
    expect(isStrongPassword('Aa123456')).toBe(true); // 大写 + 小写 + 数字
    expect(isStrongPassword('Ab!@#$%^')).toBe(true); // 大写 + 符号
  });

  it('长度按字符数算：多字节字符一个顶一个（与后端 chars().count() 同口径）', () => {
    expect(isStrongPassword('汉字汉字汉字汉字汉字汉字汉字汉字')).toBe(false); // 8 个汉字，单类
    expect(isStrongPassword('汉字汉字汉字汉字汉字汉字汉字汉字1')).toBe(true); // 汉字算「其它」+ 数字
  });
});
