import { Card, Descriptions, Tag } from 'antd';
import { useAuth } from '../../auth/AuthContext';

export default function Dashboard() {
  const { user, perms, menus } = useAuth();
  return (
    <Card title="欢迎回来">
      <Descriptions column={1} bordered size="small" style={{ maxWidth: 520 }}>
        <Descriptions.Item label="用户名">{user?.username}</Descriptions.Item>
        <Descriptions.Item label="昵称">{user?.nickname || '-'}</Descriptions.Item>
        <Descriptions.Item label="超级管理员">{user?.is_super ? <Tag color="gold">是</Tag> : '否'}</Descriptions.Item>
        <Descriptions.Item label="权限码数量">{perms.length}</Descriptions.Item>
        <Descriptions.Item label="可访问菜单数">{menus.length}</Descriptions.Item>
      </Descriptions>
    </Card>
  );
}
