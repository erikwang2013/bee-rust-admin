import {
  ApiOutlined,
  ApartmentOutlined,
  AppstoreOutlined,
  AreaChartOutlined,
  AuditOutlined,
  BarChartOutlined,
  BellOutlined,
  CalendarOutlined,
  CheckCircleOutlined,
  ClockCircleOutlined,
  CloseCircleOutlined,
  ClusterOutlined,
  ControlOutlined,
  DashboardOutlined,
  DatabaseOutlined,
  DeleteOutlined,
  DeploymentUnitOutlined,
  DownloadOutlined,
  EditOutlined,
  ExclamationCircleOutlined,
  FileOutlined,
  FileSearchOutlined,
  FileTextOutlined,
  FilterOutlined,
  FolderOpenOutlined,
  FolderOutlined,
  FundOutlined,
  GlobalOutlined,
  HomeOutlined,
  IdcardOutlined,
  InfoCircleOutlined,
  KeyOutlined,
  LineChartOutlined,
  LockOutlined,
  LoginOutlined,
  MailOutlined,
  MenuOutlined,
  MessageOutlined,
  NotificationOutlined,
  PictureOutlined,
  PieChartOutlined,
  PlusOutlined,
  ProfileOutlined,
  SafetyOutlined,
  SearchOutlined,
  SettingOutlined,
  ShopOutlined,
  ShoppingOutlined,
  SolutionOutlined,
  StarOutlined,
  TagsOutlined,
  TeamOutlined,
  ThunderboltOutlined,
  ToolOutlined,
  UnlockOutlined,
  UploadOutlined,
  UsergroupAddOutlined,
  UserOutlined,
  VideoCameraOutlined,
  WarningOutlined,
} from '@ant-design/icons';
import { Suspense, createElement, lazy, type ComponentType } from 'react';

/**
 * 菜单 icon 字符串 → 图标组件。
 *
 * 这里刻意用手写映射取代曾经的 `import * as Icons from '@ant-design/icons'`：
 * 通配导入会把整包 836 个图标模块全拖进 antd 分包，而 antd 分包是**登录页首屏
 * 就要下载**的（改动前 ~2MB / gzip 560kB）。菜单 icon 是运维在「菜单管理」里
 * 手填的字符串，实际高频使用的就下面这些，所以只静态引入它们，其余名字走懒加载。
 *
 * 增删条目只影响首屏体积，不影响功能：没列进来的名字照样能渲染（见 menuIcon）。
 */
const ICONS: Record<string, ComponentType> = {
  // 种子菜单用到的七个（见 menu 表），勿删
  SettingOutlined,
  UserOutlined,
  TeamOutlined,
  MenuOutlined,
  ApartmentOutlined,
  LoginOutlined,
  FileSearchOutlined,
  // 概览 / 首页
  AppstoreOutlined,
  DashboardOutlined,
  HomeOutlined,
  FundOutlined,
  BarChartOutlined,
  // 人员 / 组织
  ProfileOutlined,
  IdcardOutlined,
  UsergroupAddOutlined,
  SolutionOutlined,
  AuditOutlined,
  // 安全 / 权限
  SafetyOutlined,
  LockOutlined,
  UnlockOutlined,
  KeyOutlined,
  ControlOutlined,
  // 数据 / 文件
  DatabaseOutlined,
  FileOutlined,
  FileTextOutlined,
  FolderOutlined,
  FolderOpenOutlined,
  // 图表
  LineChartOutlined,
  PieChartOutlined,
  AreaChartOutlined,
  // 通知 / 消息
  BellOutlined,
  NotificationOutlined,
  MailOutlined,
  MessageOutlined,
  // 检索
  SearchOutlined,
  FilterOutlined,
  TagsOutlined,
  // 网络 / 服务
  GlobalOutlined,
  ApiOutlined,
  DeploymentUnitOutlined,
  ClusterOutlined,
  // 业务 / 时间 / 媒体
  ShopOutlined,
  ShoppingOutlined,
  CalendarOutlined,
  ClockCircleOutlined,
  PictureOutlined,
  VideoCameraOutlined,
  // 增删改查
  EditOutlined,
  DeleteOutlined,
  PlusOutlined,
  UploadOutlined,
  DownloadOutlined,
  // 状态
  CheckCircleOutlined,
  CloseCircleOutlined,
  ExclamationCircleOutlined,
  InfoCircleOutlined,
  WarningOutlined,
  // 杂项
  ToolOutlined,
  ThunderboltOutlined,
  StarOutlined,
};

/** 名字 → 兜底组件。同一个名字只构建一次，重复渲染拿到的是同一引用。 */
const fallbackCache = new Map<string, ComponentType>();

function unmappedIcon(name: string): ComponentType {
  let Cmp = fallbackCache.get(name);
  if (!Cmp) {
    const Lazy = lazy(() =>
      import('@ant-design/icons').then((m) => ({
        // 库里根本没有这个名字也仍然给个能渲染的默认图标，不抛错
        default: (m as unknown as Record<string, ComponentType>)[name] ?? AppstoreOutlined,
      })),
    );
    // Suspense 放在图标自己内部，调用方（侧边栏）不必再包一层边界；
    // 懒加载分包没到位时先渲染空，到位后自动补上
    Cmp = () => createElement(Suspense, { fallback: null }, createElement(Lazy));
    fallbackCache.set(name, Cmp);
  }
  return Cmp;
}

/**
 * 菜单 icon 字符串 → 可 `<Icon />` 渲染的组件。
 *
 * 映射内有则直接返回（同步、零额外请求）；没有则返回懒加载包装组件。
 * 无名字或不认识的名字都回落到默认图标。
 */
export function menuIcon(name?: string): ComponentType {
  if (!name) return AppstoreOutlined;
  return ICONS[name] ?? unmappedIcon(name);
}
