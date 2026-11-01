import { describe, expect, it } from 'vitest';
import AppstoreOutlined from '@ant-design/icons/es/icons/AppstoreOutlined';
import SettingOutlined from '@ant-design/icons/es/icons/SettingOutlined';
import { menuIcon } from './icons';

describe('menuIcon', () => {
  it('映射里的名字同步取到静态引入的组件', () => {
    expect(menuIcon('SettingOutlined')).toBe(SettingOutlined);
  });

  it('空名字回落默认图标', () => {
    expect(menuIcon()).toBe(AppstoreOutlined);
    expect(menuIcon('')).toBe(AppstoreOutlined);
  });

  it('映射外的名字不抛错，统一回落到 AppstoreOutlined', () => {
    const a = menuIcon('DefinitelyNotAnIcon');
    expect(a).toBe(AppstoreOutlined); // 能直接 <Icon /> 渲染
    expect(menuIcon('DefinitelyNotAnIcon')).toBe(a);
    expect(menuIcon('SomeOtherUnknownIcon')).toBe(a);
  });
});
