import { useCallback, useEffect, useState } from 'react';
import {
  App, Button, Form, Input, Modal, Popconfirm, Select, Space, Switch, Table, Tag,
} from 'antd';
import { DownloadOutlined, PlusOutlined, ReloadOutlined } from '@ant-design/icons';
import type { ColumnsType } from 'antd/es/table';
import { adminApi, type AdminForm, type AdminQuery } from '../../../api/admin';
import { downloadCsv } from '../../../api/download';
import { roleApi } from '../../../api/role';
import type { Admin, Role } from '../../../api/types';
import { deptApi } from '../../../api/dept';
import type { Dept } from '../../../api/types';
import Auth from '../../../auth/Auth';
import { useAuth } from '../../../auth/AuthContext';
import { useI18n } from '../../../i18n';

/** 部门树 → 扁平选项（带父级路径，子部门也能选到）。 */
function toDeptOptions(nodes: Dept[], prefix = ''): { value: number; label: string }[] {
  return nodes.flatMap((d) => {
    const label = prefix ? `${prefix} / ${d.name}` : d.name;
    return [{ value: d.id, label }, ...toDeptOptions(d.children ?? [], label)];
  });
}

export default function AdminPage() {
  const { message } = App.useApp();
  const { user: me } = useAuth();
  const { t } = useI18n();
  const [form] = Form.useForm<AdminForm>();
  const [query, setQuery] = useState<AdminQuery>({ page: 1, size: 10 });
  const [rows, setRows] = useState<Admin[]>([]);
  const [total, setTotal] = useState(0);
  const [loading, setLoading] = useState(false);
  const [roles, setRoles] = useState<Role[]>([]);
  const [depts, setDepts] = useState<Dept[]>([]);
  const [modalOpen, setModalOpen] = useState(false);
  const [editing, setEditing] = useState<Admin | null>(null);

  const load = useCallback(async (q: AdminQuery) => {
    setLoading(true);
    try {
      const res = await adminApi.list(q);
      setRows(res.list);
      setTotal(res.total);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { void load(query); }, [query, load]);
  useEffect(() => {
    void roleApi.list({ page: 1, size: 200 }).then((r) => setRoles(r.list));
    void deptApi.tree().then(setDepts);
  }, []);

  const openCreate = () => {
    setEditing(null);
    form.resetFields();
    form.setFieldsValue({ status: 1, sex: 0, dept_id: 0, role_ids: [] } as unknown as AdminForm);
    setModalOpen(true);
  };

  const openEdit = async (row: Admin) => {
    const detail = await adminApi.get(row.id);
    setEditing(detail);
    form.setFieldsValue({
      nickname: detail.nickname, email: detail.email, phone: detail.phone, sex: detail.sex,
      dept_id: detail.dept_id, status: detail.status, remark: detail.remark,
      role_ids: detail.role_ids ?? [],
    } as unknown as AdminForm);
    setModalOpen(true);
  };

  const submit = async () => {
    const v = await form.validateFields();
    if (editing) {
      await adminApi.update(editing.id, v);
      message.success(t('common.saved'));
    } else {
      await adminApi.create(v);
      message.success(t('common.created'));
    }
    setModalOpen(false);
    void load(query);
  };

  const columns: ColumnsType<Admin> = [
    { title: t('field.id'), dataIndex: 'id', width: 70 },
    { title: t('field.username'), dataIndex: 'username' },
    { title: t('field.nickname'), dataIndex: 'nickname' },
    { title: t('field.dept'), dataIndex: 'dept_name', render: (v) => v || '-' },
    {
      title: t('field.roles'), dataIndex: 'role_names',
      render: (v: string[], row) =>
        row.is_super ? <Tag color="gold">{t('admin.super')}</Tag> : (v ?? []).map((n) => <Tag key={n}>{n}</Tag>),
    },
    {
      title: t('field.status'), dataIndex: 'status', width: 90,
      render: (v: number, row) => (
        <Auth code="system:admin:edit">
          <Switch
            checked={v === 1}
            disabled={row.id === me?.id || row.is_super}
            onChange={async (checked) => {
              await adminApi.setStatus(row.id, checked ? 1 : 0);
              message.success(t('common.updated'));
              void load(query);
            }}
          />
        </Auth>
      ),
    },
    { title: t('field.created_at'), dataIndex: 'created_at', width: 170 },
    {
      title: t('common.actions'), width: 240, fixed: 'right',
      render: (_, row) => (
        <Space>
          <Auth code="system:admin:edit">
            <Button size="small" type="link" onClick={() => void openEdit(row)}>{t('common.edit')}</Button>
          </Auth>
          <Auth code="system:admin:resetPwd">
            <Button
              size="small" type="link"
              disabled={row.is_super && row.id !== me?.id}
              onClick={() => {
                let pwd = '';
                Modal.confirm({
                  title: t('admin.reset_pwd_title', { username: row.username }),
                  content: (
                    <Input.Password placeholder={t('admin.reset_pwd_placeholder')} onChange={(e) => { pwd = e.target.value; }} />
                  ),
                  onOk: async () => {
                    if (pwd.length < 6) {
                      message.error(t('validate.min_len', { field: t('field.password'), n: 6 }));
                      return Promise.reject(new Error('too short'));
                    }
                    await adminApi.resetPassword(row.id, pwd);
                    message.success(t('admin.reset_done'));
                  },
                });
              }}
            >
              {t('admin.reset_pwd')}
            </Button>
          </Auth>
          <Auth code="system:admin:remove">
            <Popconfirm
              title={t('admin.delete_confirm')}
              onConfirm={async () => { await adminApi.remove(row.id); message.success(t('common.deleted')); void load(query); }}
            >
              <Button
                size="small" type="link" danger
                disabled={row.id === me?.id || row.is_super}
              >
                {t('common.delete')}
              </Button>
            </Popconfirm>
          </Auth>
        </Space>
      ),
    },
  ];

  // 只有真存在授不出去的角色时才提示；超管（都能授）不该看到这句话
  const hasUngrantable = roles.some((r) => r.grantable === false);

  return (
    <>
      <Space style={{ marginBottom: 16 }} wrap>
        <Input.Search
          placeholder={t('field.username')} allowClear style={{ width: 200 }}
          onSearch={(v) => setQuery((q) => ({ ...q, username: v || undefined, page: 1 }))}
        />
        <Select
          placeholder={t('field.status')} allowClear style={{ width: 120 }}
          options={[{ value: 1, label: t('common.enabled') }, { value: 0, label: t('common.disabled') }]}
          onChange={(v) => setQuery((q) => ({ ...q, status: v, page: 1 }))}
        />
        <Button icon={<ReloadOutlined />} onClick={() => void load(query)}>{t('common.refresh')}</Button>
        <Button icon={<DownloadOutlined />} onClick={() => void downloadCsv('/admins/export', query, 'admins')}>
          {t('common.export')}
        </Button>
        <Auth code="system:admin:add">
          <Button type="primary" icon={<PlusOutlined />} onClick={openCreate}>{t('common.add')}</Button>
        </Auth>
      </Space>

      <Table<Admin>
        rowKey="id"
        size="small"
        loading={loading}
        columns={columns}
        dataSource={rows}
        scroll={{ x: 1100 }}
        pagination={{
          current: query.page, pageSize: query.size, total, showSizeChanger: true,
          onChange: (page, size) => setQuery((q) => ({ ...q, page, size })),
        }}
      />

      <Modal
        title={editing ? t('admin.edit_title', { username: editing.username }) : t('admin.create_title')}
        open={modalOpen}
        onCancel={() => setModalOpen(false)}
        onOk={() => void submit()}
        destroyOnClose
        width={560}
      >
        <Form form={form} labelCol={{ span: 5 }} wrapperCol={{ span: 18 }}>
          {!editing && (
            <>
              <Form.Item
                name="username" label={t('field.username')}
                rules={[{ required: true, min: 3, message: t('validate.min_len', { field: t('field.username'), n: 3 }) }]}
              >
                <Input autoComplete="off" />
              </Form.Item>
              <Form.Item
                name="password" label={t('field.initial_password')}
                rules={[{ required: true, min: 6, message: t('validate.min_len', { field: t('field.password'), n: 6 }) }]}
              >
                <Input.Password autoComplete="new-password" />
              </Form.Item>
            </>
          )}
          <Form.Item
            name="nickname" label={t('field.nickname')}
            rules={[{ required: true, message: t('validate.required', { field: t('field.nickname') }) }]}
          >
            <Input />
          </Form.Item>
          <Form.Item name="dept_id" label={t('field.dept')}>
            <Select
              allowClear placeholder={t('admin.pick_dept')}
              options={[{ value: 0, label: t('common.none') }, ...toDeptOptions(depts)]}
            />
          </Form.Item>
          <Form.Item
            name="role_ids" label={t('field.roles')}
            extra={hasUngrantable ? t('admin.grant_hint') : undefined}
          >
            <Select
              mode="multiple" allowClear placeholder={t('admin.pick_roles')}
              // 授不出去的角色不隐藏、只禁用：让人看见它存在，也知道为什么点不动。
              // 实测：已选中的禁用项仍显示为已选（编辑别人授过宽角色的人不会丢），
              // 但标签上没有 ×，要清掉只能走选择框的 allowClear。
              options={roles.map((r) => ({ value: r.id, label: r.name, disabled: r.grantable === false }))}
            />
          </Form.Item>
          <Form.Item name="email" label={t('field.email')}><Input /></Form.Item>
          <Form.Item name="phone" label={t('field.phone')}><Input /></Form.Item>
          <Form.Item name="sex" label={t('field.sex')}>
            <Select options={[
              { value: 0, label: t('field.sex_unknown') },
              { value: 1, label: t('field.sex_male') },
              { value: 2, label: t('field.sex_female') },
            ]} />
          </Form.Item>
          <Form.Item name="status" label={t('field.status')}>
            <Select options={[{ value: 1, label: t('common.enabled') }, { value: 0, label: t('common.disabled') }]} />
          </Form.Item>
          <Form.Item name="remark" label={t('field.remark')}><Input.TextArea rows={2} /></Form.Item>
        </Form>
      </Modal>
    </>
  );
}
