import { useCallback, useEffect, useMemo, useState } from 'react';
import {
  App, Button, Card, Empty, Form, Input, Modal, Popconfirm, Select, Space, Switch, Tag, Typography,
} from 'antd';
import { PlusOutlined, ReloadOutlined } from '@ant-design/icons';
import { configApi, type ConfigForm } from '../../../api/config';
import type { SysConfig } from '../../../api/types';
import Auth from '../../../auth/Auth';
import { useI18n, type TFunc } from '../../../i18n';

/** 分组名 → 词表键；未知分组原样显示（分组是数据，不强制翻译） */
const GROUP_TEXT: Record<string, string> = {
  site: 'config.group_site',
};

function groupLabel(name: string, t: TFunc): string {
  if (name === '') return t('config.group_default');
  const key = GROUP_TEXT[name];
  return key ? t(key as Parameters<TFunc>[0]) : name;
}

/** 表单里 bool 键用真布尔（Switch），其余字符串 */
type ConfigFormValues = Record<string, string | boolean>;

const toControl = (it: SysConfig): string | boolean =>
  it.value_type === 'bool' ? it.value === 'true' || it.value === '1' : it.value;

const controlToValue = (it: SysConfig, v: string | boolean | undefined): string =>
  it.value_type === 'bool' ? (v ? 'true' : 'false') : String(v ?? '');

export default function ConfigPage() {
  const { message } = App.useApp();
  const { t } = useI18n();
  const [rows, setRows] = useState<SysConfig[]>([]);
  const [loading, setLoading] = useState(false);
  const [open, setOpen] = useState(false);
  const [creating, setCreating] = useState(false);
  const [form] = Form.useForm<ConfigForm>();

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const res = await configApi.list();
      setRows(res.list);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { void load(); }, [load]);

  const groups = useMemo(() => {
    const m = new Map<string, SysConfig[]>();
    for (const r of rows) {
      const g = r.group || '';
      if (!m.has(g)) m.set(g, []);
      m.get(g)!.push(r);
    }
    return [...m.entries()].sort((a, b) => a[0].localeCompare(b[0]));
  }, [rows]);

  const remove = async (row: SysConfig) => {
    await configApi.remove(row.id);
    message.success(t('common.deleted'));
    void load();
  };

  const openCreate = () => {
    form.resetFields();
    form.setFieldsValue({ value_type: 'string', value: '', group: '', is_public: 0, remark: '' });
    setOpen(true);
  };

  const submitCreate = async () => {
    const v = await form.validateFields();
    setCreating(true);
    try {
      await configApi.create(v);
      message.success(t('common.created'));
      setOpen(false);
      void load();
    } finally {
      setCreating(false);
    }
  };

  return (
    <>
      <Space style={{ marginBottom: 12 }} wrap>
        <Button icon={<ReloadOutlined />} loading={loading} onClick={() => void load()}>
          {t('common.refresh')}
        </Button>
        <Auth code="system:config:edit">
          <Button type="primary" icon={<PlusOutlined />} onClick={openCreate}>
            {t('config.new')}
          </Button>
        </Auth>
      </Space>
      <Typography.Paragraph type="secondary" style={{ marginBottom: 12 }}>
        {t('config.hint')}
      </Typography.Paragraph>

      {rows.length === 0 && !loading ? (
        <Empty description={t('config.empty')} />
      ) : (
        groups.map(([name, items]) => (
          <GroupCard key={name} name={name} items={items} onChanged={load} onDelete={remove} />
        ))
      )}

      <Modal
        title={t('config.create_title')}
        open={open}
        onCancel={() => setOpen(false)}
        onOk={() => void submitCreate()}
        confirmLoading={creating}
        width={480}
      >
        <Form form={form} labelCol={{ span: 6 }} wrapperCol={{ span: 17 }}>
          <Form.Item
            name="key"
            label={t('config.key')}
            rules={[
              { required: true, message: t('validate.required', { field: t('config.key') }) },
              {
                pattern: /^[a-z0-9_]+(\.[a-z0-9_]+)*$/,
                message: t('config.key_rule'),
              },
              { max: 128, message: t('config.key_rule') },
            ]}
          >
            <Input placeholder={t('config.key_example')} />
          </Form.Item>
          <Form.Item
            name="group"
            label={t('config.group')}
            rules={[{ pattern: /^[a-z0-9_]*$/, message: t('config.group_rule') }, { max: 64 }]}
          >
            <Input placeholder={t('config.group_example')} />
          </Form.Item>
          <Form.Item name="value_type" label={t('field.type')}>
            <Select
              options={[
                { value: 'string', label: t('config.type_string') },
                { value: 'text', label: t('config.type_text') },
                { value: 'bool', label: t('config.type_bool') },
              ]}
            />
          </Form.Item>
          <Form.Item name="value" label={t('field.value')}>
            <Input.TextArea rows={2} placeholder={t('config.value_placeholder')} />
          </Form.Item>
          <Form.Item name="is_public" label={t('config.public')} valuePropName="checked">
            <Switch />
          </Form.Item>
          <Form.Item name="remark" label={t('field.remark')}>
            <Input placeholder={t('config.remark_placeholder')} />
          </Form.Item>
        </Form>
      </Modal>
    </>
  );
}

function GroupCard({
  name, items, onChanged, onDelete,
}: {
  name: string;
  items: SysConfig[];
  onChanged: () => void;
  onDelete: (row: SysConfig) => void;
}) {
  const { message } = App.useApp();
  const { t } = useI18n();
  const [form] = Form.useForm<ConfigFormValues>();
  const [saving, setSaving] = useState(false);

  // items 变化（保存后重载/删除）时把表单值同步回服务端最新值
  useEffect(() => {
    const v: ConfigFormValues = {};
    for (const it of items) v[it.key] = toControl(it);
    form.setFieldsValue(v);
  }, [items, form]);

  /** 只提交改动过的项：全量 PUT 会在「只改了 A、B 的服务端值恰好也变了」时互相覆盖 */
  const save = async () => {
    const vals = form.getFieldsValue();
    const changed = items.filter((it) => controlToValue(it, vals[it.key]) !== it.value);
    if (changed.length === 0) {
      message.info(t('config.no_change'));
      return;
    }
    setSaving(true);
    try {
      for (const it of changed) {
        await configApi.update(it.id, { value: controlToValue(it, vals[it.key]) });
      }
      message.success(t('common.saved'));
      onChanged();
    } finally {
      setSaving(false);
    }
  };

  return (
    <Card
      size="small"
      title={groupLabel(name, t)}
      style={{ maxWidth: 720, marginBottom: 12 }}
      extra={
        <Auth code="system:config:edit">
          <Button size="small" type="primary" loading={saving} onClick={() => void save()}>
            {t('common.save')}
          </Button>
        </Auth>
      }
    >
      <Form form={form} layout="vertical">
        {items.map((it) => (
          <Form.Item
            key={it.id}
            name={it.key}
            valuePropName={it.value_type === 'bool' ? 'checked' : 'value'}
            label={
              <Space size={6} wrap>
                <span>{it.remark || it.key}</span>
                {it.is_public === 1 && <Tag color="green">{t('config.public')}</Tag>}
              </Space>
            }
            style={{ marginBottom: 16 }}
            extra={
              <Space size={8}>
                <Typography.Text type="secondary" style={{ fontSize: 12 }}>
                  {it.key}
                </Typography.Text>
                <Auth code="system:config:edit">
                  <Popconfirm
                    title={t('config.delete_confirm', { key: it.key })}
                    description={t('config.delete_desc')}
                    onConfirm={() => onDelete(it)}
                  >
                    <Button type="link" size="small" danger>
                      {t('common.delete')}
                    </Button>
                  </Popconfirm>
                </Auth>
              </Space>
            }
          >
            {it.value_type === 'bool' ? (
              <Switch />
            ) : it.value_type === 'text' ? (
              <Input.TextArea rows={2} />
            ) : (
              <Input />
            )}
          </Form.Item>
        ))}
      </Form>
    </Card>
  );
}
