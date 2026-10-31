import { useCallback, useEffect, useMemo, useState } from 'react';
import {
  App, Button, Card, Empty, Form, Input, Modal, Popconfirm, Select, Space, Switch, Tag, Typography,
} from 'antd';
import PlusOutlined from '@ant-design/icons/es/icons/PlusOutlined';
import ReloadOutlined from '@ant-design/icons/es/icons/ReloadOutlined';
import { configApi, type ConfigForm, type ConfigUpdateForm } from '../../../api/config';
import type { SysConfig } from '../../../api/types';
import Auth from '../../../auth/Auth';
import { useI18n, type TFunc } from '../../../i18n';
import { configPatch } from './patch';

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

/**
 * 新增弹窗的表单值：`is_public` 是 Switch，表单里必须是 **boolean**；
 * 提交时归一成后端的 0/1 整数。类型上就与线上的 `ConfigForm.is_public: number` 错开，
 * 漏了归一化会直接编译不过（之前用 number 谎报，布尔塞进 i8 必 400）。
 */
type ConfigCreateValues = Omit<ConfigForm, 'is_public'> & { is_public: boolean };

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
  const [form] = Form.useForm<ConfigCreateValues>();

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
    form.setFieldsValue({ value_type: 'string', value: '', group: '', is_public: false, remark: '' });
    setOpen(true);
  };

  const submitCreate = async () => {
    const v = await form.validateFields();
    setCreating(true);
    try {
      // Switch 的布尔归一成后端要的 0/1 整数（类型上 ConfigCreateValues 已强制这一步）
      await configApi.create({ ...v, is_public: v.is_public ? 1 : 0 });
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
              // 与后端 valid_key 的 2..=128 对齐：前端放过 1 位键只会换来一次 400
              { min: 2, max: 128, message: t('config.key_rule') },
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
  /**
   * 公开开关不挂在 Form 里，用显式 state：开关在每项的 `extra` 栏（值控件旁边），
   * 与「值」是两个独立字段。这样「值改了但开关没动」「只拨开关」都只提交各自那半边
   * （后端 PUT 本来就是部分更新），也不用依赖嵌套 Form.Item 的字段注册行为。
   */
  const [publics, setPublics] = useState<Record<string, boolean>>({});

  // items 变化（保存后重载/删除）时把表单值与公开开关都同步回服务端最新值
  useEffect(() => {
    const v: ConfigFormValues = {};
    const p: Record<string, boolean> = {};
    for (const it of items) {
      v[it.key] = toControl(it);
      p[it.key] = it.is_public === 1;
    }
    form.setFieldsValue(v);
    setPublics(p);
  }, [items, form]);

  /** 只提交改动过的项（值与公开开关各自比较）：全量 PUT 会在「只改了 A、B 的服务端值
   *  恰好也变了」时互相覆盖；开关也走部分更新，不用「删了重建」。 */
  const save = async () => {
    const vals = form.getFieldsValue();
    const changed: { it: SysConfig; patch: ConfigUpdateForm }[] = [];
    for (const it of items) {
      const patch = configPatch(it, controlToValue(it, vals[it.key]), publics[it.key] === true);
      if (patch) changed.push({ it, patch });
    }
    if (changed.length === 0) {
      message.info(t('config.no_change'));
      return;
    }
    setSaving(true);
    let failed = 0;
    try {
      for (const { it, patch } of changed) {
        try {
          await configApi.update(it.id, patch);
        } catch {
          // 单项失败（如 notify.* 不许公开）不拖垮其余项；具体原因 http 拦截器已弹过
          failed += 1;
        }
      }
    } finally {
      setSaving(false);
    }
    if (failed > 0) {
      message.warning(t('config.save_partial', { n: failed }));
    } else {
      message.success(t('common.saved'));
    }
    // 成败都重载：失败项的开关/输入回滚到服务端值，页面不会停在「看起来改成功了」
    onChanged();
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
              <Space size={8} wrap>
                <Typography.Text type="secondary" style={{ fontSize: 12 }}>
                  {it.key}
                </Typography.Text>
                <Auth code="system:config:edit">
                  <Space size={4}>
                    <Switch
                      size="small"
                      checked={publics[it.key] === true}
                      onChange={(v) => setPublics((p) => ({ ...p, [it.key]: v }))}
                    />
                    <Typography.Text type="secondary" style={{ fontSize: 12 }}>
                      {t('config.public')}
                    </Typography.Text>
                  </Space>
                </Auth>
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
