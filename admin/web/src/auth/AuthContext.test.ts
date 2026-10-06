import { describe, expect, it } from 'vitest';
import { avatarUrl } from './AuthContext';

describe('avatarUrl', () => {
  it('有头像时拼版本号（否则换头像后浏览器一直吃旧缓存）', () => {
    expect(avatarUrl('/avatar/7', 3)).toBe('/avatar/7?v=3');
  });

  it('version 变化 → URL 跟着变', () => {
    expect(avatarUrl('/avatar/7', 3)).not.toBe(avatarUrl('/avatar/7', 4));
  });

  it('没头像回落看板宠物图，不带查询串', () => {
    expect(avatarUrl(undefined, 9)).toBe('/keeper-head.svg');
    expect(avatarUrl('', 9)).toBe('/keeper-head.svg');
  });
});
