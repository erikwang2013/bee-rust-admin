import { describe, expect, it } from 'vitest';
import { FALLBACK_REPO, copyrightText, safeRepoUrl, str } from './siteinfo';

describe('str', () => {
  it('只认非空字符串，空白/布尔/undefined 都可以当没配', () => {
    expect(str('x')).toBe('x');
    expect(str('  ')).toBeNull();
    expect(str('')).toBeNull();
    expect(str(true)).toBeNull();
    expect(str(false)).toBeNull();
    expect(str(undefined)).toBeNull();
  });
});

describe('safeRepoUrl', () => {
  it('http/https 原样放行（大小写不敏感）', () => {
    expect(safeRepoUrl('https://github.com/erikwang2013/bee-rust-admin')).toBe(
      'https://github.com/erikwang2013/bee-rust-admin',
    );
    expect(safeRepoUrl('http://example.com')).toBe('http://example.com');
    expect(safeRepoUrl('HTTPS://Example.com/x')).toBe('HTTPS://Example.com/x');
  });

  it('非 http(s) scheme 一律回落内置地址（存储型 XSS 面：这个值进未登录页的 <a href>）', () => {
    expect(safeRepoUrl('javascript:alert(1)')).toBe(FALLBACK_REPO);
    expect(safeRepoUrl('JavaScript:alert(1)')).toBe(FALLBACK_REPO);
    expect(safeRepoUrl('data:text/html,<script>1</script>')).toBe(FALLBACK_REPO);
    expect(safeRepoUrl('vbscript:msgbox(1)')).toBe(FALLBACK_REPO);
    // 相对路径/裸域名也不放行：兜底比「猜一个基准」稳
    expect(safeRepoUrl('//evil.com')).toBe(FALLBACK_REPO);
    expect(safeRepoUrl('example.com')).toBe(FALLBACK_REPO);
  });

  it('没配/空白/非字符串走内置地址', () => {
    expect(safeRepoUrl(undefined)).toBe(FALLBACK_REPO);
    expect(safeRepoUrl('')).toBe(FALLBACK_REPO);
    expect(safeRepoUrl('   ')).toBe(FALLBACK_REPO);
    expect(safeRepoUrl(true)).toBe(FALLBACK_REPO);
  });
});

describe('copyrightText', () => {
  it('配置值优先；空白回落当前年份（注入 now 让断言稳定）', () => {
    expect(copyrightText('© 2026 验收版权', new Date(2030, 0, 1))).toBe('© 2026 验收版权');
    expect(copyrightText('', new Date(2030, 0, 1))).toBe('© 2030 erik');
    expect(copyrightText(undefined, new Date(2030, 0, 1))).toBe('© 2030 erik');
    expect(copyrightText('  ', new Date(2030, 0, 1))).toBe('© 2030 erik');
  });
});
