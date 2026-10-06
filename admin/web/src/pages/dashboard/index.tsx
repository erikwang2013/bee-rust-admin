import { Card, Descriptions, Tag } from 'antd';
import { useAuth } from '../../auth/AuthContext';
import { useI18n } from '../../i18n';

export default function Dashboard() {
  const { user, perms, menus } = useAuth();
  const { t } = useI18n();
  return (
    <Card title={t('dashboard.welcome')}>
      <Descriptions column={1} bordered size="small" style={{ maxWidth: 520 }}>
        <Descriptions.Item label={t('field.username')}>{user?.username}</Descriptions.Item>
        <Descriptions.Item label={t('field.nickname')}>{user?.nickname || '-'}</Descriptions.Item>
        <Descriptions.Item label={t('dashboard.super_admin')}>
          {user?.is_super ? <Tag color="gold">{t('common.yes')}</Tag> : t('common.no')}
        </Descriptions.Item>
        <Descriptions.Item label={t('dashboard.perm_count')}>{perms.length}</Descriptions.Item>
        <Descriptions.Item label={t('dashboard.menu_count')}>{menus.length}</Descriptions.Item>
      </Descriptions>
    </Card>
  );
}
