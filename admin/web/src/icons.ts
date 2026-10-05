import * as Icons from '@ant-design/icons';
import { AppstoreOutlined } from '@ant-design/icons';
import type { ComponentType } from 'react';

/** 菜单 icon 字符串 → antd 图标组件；不认识的名字回落默认图标。 */
export function menuIcon(name?: string): ComponentType {
  if (!name) return AppstoreOutlined;
  const Cmp = (Icons as unknown as Record<string, ComponentType>)[name];
  return Cmp ?? AppstoreOutlined;
}
