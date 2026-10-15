import { useCallback, useEffect, useState } from 'react';
import {
  App, Button, Card, Divider, Form, Input, InputNumber, Select, Space, Switch, Typography,
} from 'antd';
import { MailOutlined, ReloadOutlined } from '@ant-design/icons';
import { notifyApi, type NotifyForm } from '../../../api/notify';
import type { NotifyConfig } from '../../../api/types';
import Auth from '../../../auth/Auth';
import { useI18n } from '../../../i18n';

/**
 * 通知设置（项 6）：Webhook 出站 + SMTP 测试邮件。
 *
 * 口令的字段语义与后端逐字对齐：**框里留空 = 不提交这个字段 = 不改**（不是清空）。
 * 所以提交前要把空值删掉，不能发空串 —— 空串在后端是「清空口令」。
 */
type FormValues = Omit<NotifyForm, 'smtp_password'> & { smtp_password?: string };

const ENC_OPTIONS = [
  { value: 'starttls', label: 'STARTTLS (587)' },
  { value: 'ssl', label: 'SSL/TLS (465)' },
];

export default function NotifyPage() {
  const { message } = App.useApp();
  const { t } = useI18n();
  const [form] = Form.useForm<FormValues>();
  const [cfg, setCfg] = useState<NotifyConfig | null>(null);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const c = await notifyApi.get();
      setCfg(c);
      form.setFieldsValue({ ...c, smtp_password: undefined }); // 口令永不回显
    } finally {
      setLoading(false);
    }
  }, [form]);

  useEffect(() => { void load(); }, [load]);

  const onSubmit = async (v: FormValues) => {
    setSaving(true);
    try {
      const body: NotifyForm = { ...v };
      if (!v.smtp_password) delete body.smtp_password; // 留空 = 不改
      await notifyApi.save(body);
      message.success(t('common.saved'));
      await load();
    } finally {
      setSaving(false);
    }
  };

  /** 发测试邮件用的是**已保存**的配置（后端就是这么做的），所以这里不校验草稿表单 */
  const onTest = async () => {
    setTesting(true);
    try {
      const { to } = await notifyApi.testEmail();
      message.success(t('notify.test_sent', { to }));
    } finally {
      setTesting(false);
    }
  };

  return (
    <Card
      title={t('notify.title')}
      style={{ maxWidth: 720 }}
      loading={loading}
      extra={
        <Button icon={<ReloadOutlined />} onClick={() => void load()}>{t('common.refresh')}</Button>
      }
    >
      <Typography.Paragraph type="secondary">{t('notify.hint')}</Typography.Paragraph>

      <Form<FormValues> form={form} layout="vertical" onFinish={(v) => void onSubmit(v)}>
        <Typography.Title level={5}>Webhook</Typography.Title>
        <Form.Item name="webhook_enabled" label={t('notify.webhook_enabled')} valuePropName="checked">
          <Switch />
        </Form.Item>
        <Form.Item
          name="webhook_url"
          label={t('notify.webhook_url')}
          extra={t('notify.webhook_hint')}
          dependencies={['webhook_enabled']}
          rules={[
            ({ getFieldValue }) => ({
              validator: (_, v: string) => {
                if (!getFieldValue('webhook_enabled')) return Promise.resolve();
                if (!v) return Promise.reject(new Error(t('validate.required', { field: t('notify.webhook_url') })));
                return /^https?:\/\//.test(v)
                  ? Promise.resolve()
                  : Promise.reject(new Error(t('notify.url_rule')));
              },
            }),
          ]}
        >
          <Input placeholder="https://example.com/hook" allowClear />
        </Form.Item>

        <Divider />

        <Typography.Title level={5}>SMTP</Typography.Title>
        <Form.Item name="smtp_enabled" label={t('notify.smtp_enabled')} valuePropName="checked">
          <Switch />
        </Form.Item>
        <Form.Item
          name="smtp_host"
          label={t('notify.smtp_host')}
          dependencies={['smtp_enabled']}
          rules={[
            ({ getFieldValue }) => ({
              validator: (_, v: string) =>
                !getFieldValue('smtp_enabled') || v
                  ? Promise.resolve()
                  : Promise.reject(new Error(t('validate.required', { field: t('notify.smtp_host') }))),
            }),
          ]}
        >
          <Input placeholder="smtp.example.com" allowClear />
        </Form.Item>
        <Space size={16} align="start" wrap>
          <Form.Item name="smtp_port" label={t('notify.smtp_port')} rules={[{ required: true }]}>
            <InputNumber min={1} max={65535} style={{ width: 120 }} />
          </Form.Item>
          <Form.Item name="smtp_encryption" label={t('notify.smtp_encryption')}>
            <Select
              style={{ width: 200 }}
              options={[...ENC_OPTIONS, { value: 'none', label: t('notify.enc_none') }]}
            />
          </Form.Item>
        </Space>
        <Form.Item name="smtp_user" label={t('notify.smtp_user')}>
          <Input placeholder={t('notify.smtp_user_placeholder')} allowClear autoComplete="off" />
        </Form.Item>
        <Form.Item
          name="smtp_password"
          label={t('notify.smtp_password')}
          extra={cfg?.smtp_password_set ? t('notify.password_set_hint') : t('notify.password_unset_hint')}
        >
          <Input.Password autoComplete="new-password" />
        </Form.Item>
        <Form.Item name="smtp_from" label={t('notify.smtp_from')} rules={[{ type: 'email', message: t('validate.email') }]}>
          <Input placeholder={t('notify.smtp_from_placeholder')} allowClear />
        </Form.Item>

        <Auth code="system:notify:edit">
          <Space wrap>
            <Button type="primary" htmlType="submit" loading={saving}>{t('common.save')}</Button>
            <Button icon={<MailOutlined />} loading={testing} onClick={() => void onTest()}>
              {t('notify.test')}
            </Button>
          </Space>
          <Typography.Paragraph type="secondary" style={{ marginTop: 8, marginBottom: 0, fontSize: 12 }}>
            {t('notify.test_hint')}
          </Typography.Paragraph>
        </Auth>
      </Form>
    </Card>
  );
}
