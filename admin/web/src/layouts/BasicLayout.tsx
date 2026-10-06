import { useMemo, useState } from 'react';
import { Avatar, Breadcrumb, Dropdown, Layout, Menu, Space, theme, type MenuProps } from 'antd';
import { LogoutOutlined, UserOutlined } from '@ant-design/icons';
import { Outlet, useLocation, useNavigate } from 'react-router';
import { menuIcon } from '../icons';
import { avatarUrl, useAuth } from '../auth/AuthContext';
import { ThemeToggle } from '../theme';
import NoticeBell from './NoticeBell';
import type { MenuNode } from '../api/types';

const { Header, Sider, Content } = Layout;

type MenuItems = NonNullable<MenuProps['items']>;

function toItems(nodes: MenuNode[]): MenuItems {
  return nodes.map((n) => {
    const Icon = menuIcon(n.icon);
    const key = n.path || String(n.id);
    const kids = n.children?.length ? toItems(n.children) : undefined;
    // 有子节点走 SubMenuType（children 必填），否则平铺为 MenuItemType
    return kids
      ? { key, icon: <Icon />, label: n.name, children: kids }
      : { key, icon: <Icon />, label: n.name };
  });
}

/** key → 菜单名，用于面包屑。 */
function flatten(nodes: MenuNode[], map: Record<string, string> = {}) {
  for (const n of nodes) {
    if (n.path) map[n.path] = n.name;
    if (n.children) flatten(n.children, map);
  }
  return map;
}

export default function BasicLayout() {
  const { user, menus, logout, version } = useAuth();
  const nav = useNavigate();
  const loc = useLocation();
  const [collapsed, setCollapsed] = useState(false);
  const { token } = theme.useToken();

  const items = useMemo(() => toItems(menus), [menus]);
  const names = useMemo(() => flatten(menus), [menus]);
  const crumbs = useMemo(() => {
    const parts = loc.pathname.split('/').filter(Boolean);
    const out: string[] = [];
    let acc = '';
    for (const p of parts) {
      acc += `/${p}`;
      out.push(names[acc] ?? p);
    }
    return out;
  }, [loc.pathname, names]);

  const userMenu = {
    items: [
      { key: 'profile', icon: <UserOutlined />, label: '个人中心' },
      { type: 'divider' as const },
      { key: 'logout', icon: <LogoutOutlined />, label: '退出登录' },
    ],
    onClick: async ({ key }: { key: string }) => {
      if (key === 'profile') nav('/profile');
      if (key === 'logout') {
        await logout();
        nav('/login', { replace: true });
      }
    },
  };

  return (
    <Layout style={{ minHeight: '100vh' }}>
      <Sider collapsible collapsed={collapsed} onCollapse={setCollapsed} theme="dark" width={220}>
        <div style={{ height: 48, margin: 12, display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 8 }}>
          <span style={{ display: 'inline-flex', background: '#fff', borderRadius: '50%', padding: 2 }}>
            <img src="/keeper-head.svg" alt="阿守" width={collapsed ? 30 : 26} height={collapsed ? 30 : 26} />
          </span>
          {!collapsed && <span style={{ color: '#fff', fontWeight: 600 }}>BRD 管理后台</span>}
        </div>
        <Menu
          theme="dark"
          mode="inline"
          items={items}
          selectedKeys={[loc.pathname]}
          defaultOpenKeys={['/' + (loc.pathname.split('/')[1] ?? '')]}
          onClick={({ key }) => { if (String(key).startsWith('/')) nav(String(key)); }}
        />
      </Sider>
      <Layout>
        <Header style={{ background: token.colorBgContainer, padding: '0 16px', display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
          <Breadcrumb items={crumbs.map((c) => ({ title: c }))} />
          <Space size={16}>
            <ThemeToggle />
            <NoticeBell />
            <Dropdown menu={userMenu}>
              <span style={{ cursor: 'pointer' }}>
                <Avatar
                  size="small"
                  src={avatarUrl(user?.avatar, version)}
                  // 头像 URL 失效时（换机器/恢复备份丢了 uploads 目录）回落到阿守，而不是一个灰圆点
                  icon={<img src="/keeper-head.svg" alt="" width={22} height={22} />}
                />
                <span style={{ marginLeft: 8 }}>{user?.nickname || user?.username}</span>
              </span>
            </Dropdown>
          </Space>
        </Header>
        <Content style={{ margin: 16 }}>
          <Outlet />
        </Content>
      </Layout>
    </Layout>
  );
}
