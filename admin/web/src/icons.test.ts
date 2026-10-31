import { describe, expect, it } from 'vitest';
import AppstoreOutlined from '@ant-design/icons/es/icons/AppstoreOutlined';
import SettingOutlined from '@ant-design/icons/es/icons/SettingOutlined';
import { loadIcon, menuIcon } from './icons';

describe('menuIcon', () => {
  it('映射里的名字同步取到静态引入的组件', () => {
    expect(menuIcon('SettingOutlined')).toBe(SettingOutlined);
  });

  it('空名字回落默认图标', () => {
    expect(menuIcon()).toBe(AppstoreOutlined);
    expect(menuIcon('')).toBe(AppstoreOutlined);
  });

  it('映射外的名字不抛错，同一个名字永远给同一个组件（懒加载只建一次）', () => {
    const a = menuIcon('DefinitelyNotAnIcon');
    expect(typeof a).toBe('function'); // 能直接 <Icon /> 渲染
    // 每次重新 lazy() 会得到新组件类型，React 会当成换了个组件整体重挂载
    expect(menuIcon('DefinitelyNotAnIcon')).toBe(a);
    expect(menuIcon('SomeOtherUnknownIcon')).not.toBe(a);
  });
});

describe('loadIcon', () => {
  // 冷启动时要现转一次图标模块，放宽超时避免 CI 上首次加载就判失败
  it('映射外但图标包里有这个名字时，解析到它自己的模块', async () => {
    const { default: AlertOutlined } = await import('@ant-design/icons/es/icons/AlertOutlined');
    const { default: resolved } = await loadIcon('AlertOutlined');
    expect(resolved).toBe(AlertOutlined);
  }, 30_000);

  it('图标包里没有的名字才回落到默认图标', async () => {
    const { default: resolved } = await loadIcon('DefinitelyNotAnIcon');
    expect(resolved).toBe(AppstoreOutlined);
  });
});
