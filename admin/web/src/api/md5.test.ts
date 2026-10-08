import { describe, expect, it } from 'vitest';
import { md5Hex } from './md5';

const utf8 = (s: string) => new TextEncoder().encode(s);

describe('md5Hex', () => {
  // RFC 1321 A.5 的用例
  it('RFC 1321 已知向量', () => {
    expect(md5Hex(utf8(''))).toBe('d41d8cd98f00b204e9800998ecf8427e');
    expect(md5Hex(utf8('a'))).toBe('0cc175b9c0f1b6a831c399e269772661');
    expect(md5Hex(utf8('abc'))).toBe('900150983cd24fb0d6963f7d28e17f72');
    expect(md5Hex(utf8('message digest'))).toBe('f96b697d7cb7938d525a2f31aaf161d0');
    expect(md5Hex(utf8('abcdefghijklmnopqrstuvwxyz'))).toBe('c3fcd3d76192e4007dfb496cca67e13b');
    expect(md5Hex(utf8('ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789'))).toBe(
      'd174ab98d277d9f5a5611c2c9f419d9f',
    );
    expect(
      md5Hex(utf8('12345678901234567890123456789012345678901234567890123456789012345678901234567890')),
    ).toBe('57edf4a22be3c955ac49da2e2107b67a');
  });

  // 期望值由 `md5sum` / openssl 独立算出（填充边界 55/56/57/64/65 都在其中）
  it('跨块与填充边界的已知向量', () => {
    const n = 200;
    const pattern = new Uint8Array(n);
    for (let i = 0; i < n; i++) pattern[i] = (i * 37 + n) & 0xff;
    expect(md5Hex(pattern)).toBe('1df1afcf5ad51d344266ebf2d51968e4');
    expect(md5Hex(utf8('a'.repeat(1000)))).toBe('cabe45dcc9ae5b66ba86600cca6b8ba8');
    expect(md5Hex(utf8('The quick brown fox jumps over the lazy dog'))).toBe(
      '9e107d9d372bb6826bd81d3542a419d6',
    );
  });

  it('55/56/57 与 63/64/65 字节的填充分支都给出 32 位小写十六进制', () => {
    for (const len of [55, 56, 57, 63, 64, 65]) {
      expect(md5Hex(new Uint8Array(len).fill(0x61))).toMatch(/^[0-9a-f]{32}$/);
    }
    // 55 → 56 → 57 三个长度必须互不相同（56 会多出一个填充块）
    const [a, b, c] = [55, 56, 57].map((len) => md5Hex(new Uint8Array(len).fill(0x61)));
    expect(new Set([a, b, c]).size).toBe(3);
  });

  it('不改动入参', () => {
    const input = new Uint8Array([1, 2, 3]);
    md5Hex(input);
    expect([...input]).toEqual([1, 2, 3]);
  });
});
