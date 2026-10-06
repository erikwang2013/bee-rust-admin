import { useState } from 'react';
import { Button, Card, Form, Input, theme, Typography } from 'antd';
import { LockOutlined, UserOutlined } from '@ant-design/icons';
import { useLocation, useNavigate } from 'react-router';
import { useAuth } from '../../auth/AuthContext';
import { ThemeToggle } from '../../theme';
import { LanguageToggle, useI18n } from '../../i18n';

export default function LoginPage() {
  const { login } = useAuth();
  const nav = useNavigate();
  const loc = useLocation();
  const { token } = theme.useToken();
  const { t } = useI18n();
  const [loading, setLoading] = useState(false);

  const onFinish = async (v: { username: string; password: string }) => {
    setLoading(true);
    try {
      await login(v.username, v.password);
      // 守卫写入的来路（未登录访问的页面），没有则回首页
      const from = (loc.state as { from?: string } | null)?.from;
      nav(from ?? '/', { replace: true });
    } catch {
      // 具体错误消息由 axios 拦截器提示
    } finally {
      setLoading(false);
    }
  };

  return (
    <div style={{ height: '100vh', display: 'flex', alignItems: 'center', justifyContent: 'center', background: token.colorBgLayout }}>
      <div style={{ position: 'fixed', top: 16, right: 16, display: 'flex', gap: 12 }}>
        <LanguageToggle />
        <ThemeToggle />
      </div>
      <Card style={{ width: 380 }}>
        <div style={{ textAlign: 'center' }}>
          <img src="/keeper.svg" alt={t('common.mascot_alt')} width={132} height={132} />
        </div>
        <Typography.Title level={3} style={{ textAlign: 'center', marginTop: 0, marginBottom: 24 }}>
          {t('login.title')}
        </Typography.Title>
        <Form onFinish={onFinish} size="large">
          <Form.Item name="username" rules={[{ required: true, message: t('validate.required', { field: t('field.username') }) }]}>
            <Input prefix={<UserOutlined />} placeholder={t('field.username')} autoComplete="username" />
          </Form.Item>
          <Form.Item name="password" rules={[{ required: true, message: t('validate.required', { field: t('field.password') }) }]}>
            <Input.Password prefix={<LockOutlined />} placeholder={t('field.password')} autoComplete="current-password" />
          </Form.Item>
          <Form.Item>
            <Button type="primary" htmlType="submit" block loading={loading}>
              {t('login.submit')}
            </Button>
          </Form.Item>
        </Form>
      </Card>
    </div>
  );
}
