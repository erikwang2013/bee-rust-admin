import { describe, expect, it } from 'vitest';
import { intervalText } from './format';

describe('intervalText', () => {
  it('整分钟 / 小时 / 天用大单位', () => {
    expect(intervalText('3600')).toBe('1 小时');
    expect(intervalText('86400')).toBe('1 天');
    expect(intervalText('600')).toBe('10 分钟');
    expect(intervalText('172800')).toBe('2 天');
    expect(intervalText(' 2 ')).toBe('2 秒');
  });

  it('不能整除的退回更小单位，实在不行就原样秒数', () => {
    expect(intervalText('45')).toBe('45 秒');
    expect(intervalText('5400')).toBe('90 分钟'); // 1.5 小时：整数分钟比「1.5 小时」精确
    expect(intervalText('90061')).toBe('90061 秒');
  });

  it('解析不出正整数就原样回显，不冒 NaN、不瞎猜', () => {
    // 列宽 64 是为将来换成 cron 表达式留的：那时原样显示才是对的
    expect(intervalText('0 3 * * *')).toBe('0 3 * * *');
    expect(intervalText('0')).toBe('0');
    expect(intervalText('-1')).toBe('-1');
  });

  it('空值显示占位符', () => {
    expect(intervalText('')).toBe('-');
    expect(intervalText('   ')).toBe('-');
  });
});
