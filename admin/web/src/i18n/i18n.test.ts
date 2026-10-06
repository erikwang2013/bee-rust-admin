import { describe, expect, it } from 'vitest';
import { detectLang, errText, t } from './index';
import { zhCN, type I18nKey } from './zh-CN';
import { enUS } from './en-US';

/** 文案里的 `{name}` 占位符集合。 */
const holes = (s: string) => new Set([...s.matchAll(/\{(\w+)\}/g)].map((m) => m[1]));

describe('词表完整性', () => {
  it('两种语言的键集合完全一致（多一个或少一个都不行）', () => {
    const zh = Object.keys(zhCN).sort();
    const en = Object.keys(enUS).sort();
    expect(en.filter((k) => !(k in zhCN))).toEqual([]); // 英文表没有多余键
    expect(zh.filter((k) => !(k in enUS))).toEqual([]); // 中文表没有漏译
    expect(en).toEqual(zh);
  });

  it('没有空文案', () => {
    expect(Object.entries(zhCN).filter(([, v]) => !v.trim())).toEqual([]);
    expect(Object.entries(enUS).filter(([, v]) => !v.trim())).toEqual([]);
  });

  it('同一个键的占位符两边必须一致（{n} 译成 {m} 会静默漏替换）', () => {
    for (const key of Object.keys(zhCN) as I18nKey[]) {
      expect([key, [...holes(enUS[key])].sort()]).toEqual([key, [...holes(zhCN[key])].sort()]);
    }
  });

  it('后端错误码表在词表里（键名 = err.<code>）', () => {
    for (const code of ['auth.bad_credentials', 'common.too_long', 'job.not_registered']) {
      expect(zhCN[`err.${code}` as I18nKey]).toBeTruthy();
      expect(enUS[`err.${code}` as I18nKey]).toBeTruthy();
    }
  });
});

describe('t', () => {
  it('占位替换', () => {
    expect(t('validate.required', { field: t('field.username') })).toBe('请输入用户名');
    expect(t('err.common.too_long', { field: '备注', max: 255 })).toBe('备注 长度不能超过 255 个字符');
  });

  it('缺参数时占位符原样留着，不冒 undefined', () => {
    expect(t('err.job.not_registered', {})).toBe('任务 {code} 未在代码中注册，不可手动触发');
  });

  it('键不存在返回键名本身', () => {
    expect(t('no.such.key' as I18nKey)).toBe('no.such.key');
    expect(t('err.no.such.code' as I18nKey)).toBe('err.no.such.code');
  });
});

describe('errText：后端错误码映射', () => {
  it('命中：用词表文案，忽略后端 msg', () => {
    expect(errText('auth.bad_credentials')).toBe('用户名或密码错误');
  });

  it('带参：args 填进占位符', () => {
    expect(errText('auth.throttled', { minutes: 5 })).toBe('尝试过于频繁，请 5 分钟后再试');
    expect(errText('job.not_registered', { code: 'clean_log' })).toBe('任务 clean_log 未在代码中注册，不可手动触发');
  });

  it('field 参数查字段标签；查不到就原样显示键名', () => {
    expect(errText('common.too_long', { field: 'remark', max: 255 })).toBe('备注 长度不能超过 255 个字符');
    expect(errText('common.too_long', { field: 'unknown_col', max: 10 })).toBe('unknown_col 长度不能超过 10 个字符');
  });

  it('没见过的码返回 null（调用方回落 msg）', () => {
    expect(errText('brand.new.code')).toBeNull();
    expect(errText('')).toBeNull();
  });
});

describe('detectLang：navigator.language → 语言', () => {
  it('zh 开头（含 zh-TW / zh-Hans-CN）都用中文', () => {
    expect(detectLang('zh')).toBe('zh-CN');
    expect(detectLang('zh-CN')).toBe('zh-CN');
    expect(detectLang('zh-TW')).toBe('zh-CN');
    expect(detectLang('ZH-hans-cn')).toBe('zh-CN');
  });

  it('其它语言一律英文', () => {
    expect(detectLang('en-US')).toBe('en-US');
    expect(detectLang('ja-JP')).toBe('en-US');
    expect(detectLang(' de-DE ')).toBe('en-US');
  });

  it('探测不到（空串）回落中文基准', () => {
    expect(detectLang('')).toBe('zh-CN');
    expect(detectLang('   ')).toBe('zh-CN');
  });
});
