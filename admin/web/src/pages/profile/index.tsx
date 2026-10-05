import { useState } from 'react';
import { App, Button, Card, Descriptions, Form, Input } from 'antd';
import { authApi } from '../../api/auth';
import { useAuth } from '../../auth/AuthContext';

export default function ProfilePage() {
  const { user, logout } = useAuth();
  const { message } = App.useApp();
  const [form] = Form.useForm();
  const [loading, setLoading] = useState(false);

  const onFinish = async (v: { old_password: string; new_password: string }) => {
    setLoading(true);
    try {
      await authApi.changePassword(v.old_password, v.new_password);
      message.success('密码已修改，请重新登录');
      form.resetFields();
      await logout();
      window.location.href = '/login';
    } finally {
      setLoading(false);
    }
  };

  return (
    <Card title="个人中心" style={{ maxWidth: 640 }}>
      <Descriptions column={1} size="small" style={{ marginBottom: 24 }}>
        <Descriptions.Item label="用户名">{user?.username}</Descriptions.Item>
        <Descriptions.Item label="昵称">{user?.nickname || '-'}</Descriptions.Item>
      </Descriptions>
      <Form form={form} layout="vertical" onFinish={onFinish} style={{ maxWidth: 360 }}>
        <Form.Item name="old_password" label="原密码" rules={[{ required: true, message: '请输入原密码' }]}>
          <Input.Password autoComplete="current-password" />
        </Form.Item>
        <Form.Item
          name="new_password"
          label="新密码"
          rules={[{ required: true, min: 6, message: '新密码至少 6 位' }]}
        >
          <Input.Password autoComplete="new-password" />
        </Form.Item>
        <Form.Item
          name="confirm"
          label="确认新密码"
          dependencies={['new_password']}
          rules={[
            { required: true, message: '请再次输入新密码' },
            ({ getFieldValue }) => ({
              validator: (_, value) =>
                !value || getFieldValue('new_password') === value
                  ? Promise.resolve()
                  : Promise.reject(new Error('两次输入不一致')),
            }),
          ]}
        >
          <Input.Password autoComplete="new-password" />
        </Form.Item>
        <Button type="primary" htmlType="submit" loading={loading}>保存</Button>
      </Form>
    </Card>
  );
}
