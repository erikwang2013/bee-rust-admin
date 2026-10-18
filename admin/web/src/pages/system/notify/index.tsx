import { useCallback, useEffect, useState } from 'react';
import {
  App, Button, Card, Divider, Form, Input, InputNumber, Popconfirm, Select, Space, Switch, Typography,
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
  const [clearing, setClearing] = useState(false);
  const [clearingSecret, setClearingSecret] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const c = await notifyApi.get();
      setCfg(c);
      // 两个密钥都永不回显
      form.setFieldsValue({ ...c, smtp_password: undefined, webhook_secret: undefined });
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
      if (!v.webhook_secret) delete body.webhook_secret;
      await notifyApi.save(body);
      message.success(t('common.saved'));
      await load();
    } finally {
      setSaving(false);
    }
  };

  /** 清空口令：显式提交空串（后端的清空语义）。拼体用的是**已保存**的配置，
   * 不拿表单草稿 —— 这个按钮只该清口令，不该顺手把没保存的改动一起存了。 */
  const clearPassword = async () => {
    if (!cfg) return;
    setClearing(true);
    try {
      const body: NotifyForm = { ...cfg, smtp_password: '' };
      await notifyApi.save(body);
      message.success(t('common.saved'));
      await load();
    } finally {
      setClearing(false);
    }
  };

  /** 清空签名密钥：与清口令同理，用已保存的配置显式提交空串 */
  const clearSecret = async () => {
    if (!cfg) return;
    setClearingSecret(true);
    try {
      const body: NotifyForm = { ...cfg, webhook_secret: '' };
      await notifyApi.save(body);
      message.success(t('common.saved'));
      await load();
    } finally {
      setClearingSecret(false);
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
                // 长度与开关无关（后端 `notify::validate` 同口径）：超长值整条进审计摘要，
                // 写不进去就丢一条审计 —— 所以关着也拦
                if ((v ?? '').length > 512) return Promise.reject(new Error(t('notify.url_too_long')));
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
        <Form.Item
          name="webhook_secret"
          label={t('notify.webhook_secret')}
          extra={cfg?.webhook_secret_set
            ? t('notify.webhook_secret_set_hint')
            : t('notify.webhook_secret_unset_hint')}
        >
          <Input.Password autoComplete="new-password" />
        </Form.Item>
        <Auth code="system:notify:edit">
          {cfg?.webhook_secret_set ? (
            <Form.Item>
              <Popconfirm
                title={t('notify.clear_secret_confirm')}
                okButtonProps={{ danger: true }}
                onConfirm={() => void clearSecret()}
              >
                <Button danger size="small" loading={clearingSecret}>{t('notify.clear_secret')}</Button>
              </Popconfirm>
            </Form.Item>
          ) : null}
        </Auth>

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
            // 255：与后端 notify::validate 同一口径（关着也拦 —— 超长值整条进审计摘要，
            // 写不进去就丢一条审计）
            { max: 255, message: t('validate.max', { max: 255 }) },
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
        <Form.Item
          name="smtp_user"
          label={t('notify.smtp_user')}
          rules={[{ max: 255, message: t('validate.max', { max: 255 }) }]}
        >
          <Input placeholder={t('notify.smtp_user_placeholder')} allowClear autoComplete="off" />
        </Form.Item>
        <Form.Item
          name="smtp_password"
          label={t('notify.smtp_password')}
          extra={cfg?.smtp_password_set ? t('notify.password_set_hint') : t('notify.password_unset_hint')}
        >
          <Input.Password autoComplete="new-password" />
        </Form.Item>
        {/* 口令框留空 = 不改（字段语义见文件头），所以「清空」得是**单独一个动作**：
            显式提交空串 + 二次确认，别让人靠猜怎么把口令删掉 */}
        <Auth code="system:notify:edit">
          {cfg?.smtp_password_set ? (
            <Form.Item>
              <Popconfirm
                title={t('notify.clear_password_confirm')}
                okButtonProps={{ danger: true }}
                onConfirm={() => void clearPassword()}
              >
                <Button danger size="small" loading={clearing}>{t('notify.clear_password')}</Button>
              </Popconfirm>
            </Form.Item>
          ) : null}
        </Auth>
        <Form.Item
          name="smtp_from"
          label={t('notify.smtp_from')}
          rules={[
            { max: 255, message: t('validate.max', { max: 255 }) },
            { type: 'email', message: t('validate.email') },
          ]}
        >
          <Input placeholder={t('notify.smtp_from_placeholder')} allowClear />
        </Form.Item>

        <Divider />

        {/* 公告邮件推送（E2-6）：开关存在 sys_config 的 notify.email_notice_enabled，
            GET /notify/config 会回包（少一个字段就是保存一次静默重置，后端有注释钉住） */}
        <Typography.Title level={5}>{t('notify.email_notice_title')}</Typography.Title>
        <Form.Item
          name="email_notice_enabled"
          label={t('notify.email_notice_enabled')}
          valuePropName="checked"
          extra={t('notify.email_notice_hint')}
        >
          <Switch />
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
