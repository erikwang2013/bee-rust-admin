import { useEffect, useState } from 'react';
import { Card, Col, Descriptions, Row, Space, Statistic, Tag, Typography } from 'antd';
import { dashboardApi } from '../../api/dashboard';
import type { DashboardStats, LoginDay } from '../../api/types';
import { useAuth } from '../../auth/AuthContext';
import { useI18n } from '../../i18n';

export default function Dashboard() {
  const { user, perms, menus } = useAuth();
  const { t } = useI18n();
  const [stats, setStats] = useState<DashboardStats | null>(null);

  useEffect(() => {
    let alive = true;
    dashboardApi
      .stats()
      .then((s) => { if (alive) setStats(s); })
      .catch(() => { /* 拿不到就只显示欢迎卡（拦截器已提示） */ });
    return () => { alive = false; };
  }, []);

  return (
    <Space orientation="vertical" size={16} style={{ display: 'flex' }}>
      <Card title={t('dashboard.welcome')} size="small">
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

      <Row gutter={[16, 16]}>
        <CountCard title={t('dashboard.admins')} value={stats?.counts.admins} />
        <CountCard title={t('dashboard.roles')} value={stats?.counts.roles} />
        <CountCard title={t('dashboard.depts')} value={stats?.counts.depts} />
        <CountCard title={t('dashboard.notices')} value={stats?.counts.notices} />
      </Row>

      <Row gutter={[16, 16]}>
        <Col xs={24} md={12}>
          <Card
            size="small"
            title={t('dashboard.login_trend')}
            extra={stats && <Typography.Text type="secondary">{t('dashboard.days_total', { n: stats.logins.success_total })}</Typography.Text>}
          >
            <MiniTrend days={stats?.logins.days} field="success" color="#1677ff" />
          </Card>
        </Col>
        <Col xs={24} md={12}>
          <Card
            size="small"
            title={t('dashboard.fail_trend')}
            extra={stats && <Typography.Text type="secondary">{t('dashboard.days_total', { n: stats.logins.fail_total })}</Typography.Text>}
          >
            <MiniTrend days={stats?.logins.days} field="fail" color="#ff4d4f" />
          </Card>
        </Col>
      </Row>
    </Space>
  );
}

function CountCard({ title, value }: { title: string; value: number | undefined }) {
  return (
    <Col xs={12} md={6}>
      <Card size="small">
        <Statistic title={title} value={value ?? '-'} />
      </Card>
    </Col>
  );
}

const W = 190; // 7 根柱 × (16 宽 + 11 间距) ≈ 189
const H = 64;

/**
 * 手绘迷你柱状趋势（不引图表库，见施工图纪律）：7 天各一根柱，柱顶标数值，
 * 底部基线 + 首末日期。全 0 时显示空态文案而不是 7 根空柱。
 * `<title>` 给悬停提示，`currentColor` 让文字跟主题走。
 */
function MiniTrend({ days, field, color }: { days: LoginDay[] | undefined; field: 'success' | 'fail'; color: string }) {
  const { t } = useI18n();
  if (!days || days.length === 0) return <Typography.Text type="secondary">{t('dashboard.no_data')}</Typography.Text>;
  const values = days.map((d) => d[field]);
  if (values.every((v) => v === 0)) return <Typography.Text type="secondary">{t('dashboard.no_data')}</Typography.Text>;
  const max = Math.max(...values, 1);

  return (
    <div>
      <svg width={W} height={H} role="img" aria-label={t('dashboard.login_trend')}>
        {days.map((d, i) => {
          const h = Math.max(2, Math.round((d[field] / max) * (H - 24)));
          const x = i * 27 + 6;
          const top = H - 10 - h;
          return (
            <g key={d.date}>
              <rect x={x} y={top} width={16} height={h} rx={2} fill={color} opacity={0.85}>
                <title>{`${d.date}: ${d[field]}`}</title>
              </rect>
              <text x={x + 8} y={top - 3} textAnchor="middle" fontSize={9} fill="currentColor" opacity={0.65}>
                {d[field]}
              </text>
            </g>
          );
        })}
        <line x1={0} y1={H - 10} x2={W} y2={H - 10} stroke="currentColor" strokeOpacity={0.2} />
      </svg>
      <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 11, opacity: 0.55 }}>
        <span>{days[0].date.slice(5)}</span>
        <span>{days[days.length - 1].date.slice(5)}</span>
      </div>
    </div>
  );
}
