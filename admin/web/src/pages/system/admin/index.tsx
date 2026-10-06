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
      message.success('已保存');
    } else {
      await adminApi.create(v);
      message.success('已创建');
    }
    setModalOpen(false);
    void load(query);
  };

  const columns: ColumnsType<Admin> = [
    { title: 'ID', dataIndex: 'id', width: 70 },
    { title: '用户名', dataIndex: 'username' },
    { title: '昵称', dataIndex: 'nickname' },
    { title: '部门', dataIndex: 'dept_name', render: (v) => v || '-' },
    {
      title: '角色', dataIndex: 'role_names',
      render: (v: string[], row) =>
        row.is_super ? <Tag color="gold">超级管理员</Tag> : (v ?? []).map((n) => <Tag key={n}>{n}</Tag>),
    },
    {
      title: '状态', dataIndex: 'status', width: 90,
      render: (v: number, row) => (
        <Auth code="system:admin:edit">
          <Switch
            checked={v === 1}
            disabled={row.id === me?.id || row.is_super}
            onChange={async (checked) => {
              await adminApi.setStatus(row.id, checked ? 1 : 0);
              message.success('已更新');
              void load(query);
            }}
          />
        </Auth>
      ),
    },
    { title: '创建时间', dataIndex: 'created_at', width: 170 },
    {
      title: '操作', width: 240, fixed: 'right',
      render: (_, row) => (
        <Space>
          <Auth code="system:admin:edit">
            <Button size="small" type="link" onClick={() => void openEdit(row)}>编辑</Button>
          </Auth>
          <Auth code="system:admin:resetPwd">
            <Button
              size="small" type="link"
              disabled={row.is_super && row.id !== me?.id}
              onClick={() => {
                let pwd = '';
                Modal.confirm({
                  title: `重置 ${row.username} 的密码`,
                  content: (
                    <Input.Password placeholder="新密码（至少 6 位）" onChange={(e) => { pwd = e.target.value; }} />
                  ),
                  onOk: async () => {
                    if (pwd.length < 6) {
                      message.error('密码至少 6 位');
                      return Promise.reject(new Error('too short'));
                    }
                    await adminApi.resetPassword(row.id, pwd);
                    message.success('已重置');
                  },
                });
              }}
            >
              重置密码
            </Button>
          </Auth>
          <Auth code="system:admin:remove">
            <Popconfirm
              title="确认删除该管理员？"
              onConfirm={async () => { await adminApi.remove(row.id); message.success('已删除'); void load(query); }}
            >
              <Button
                size="small" type="link" danger
                disabled={row.id === me?.id || row.is_super}
              >
                删除
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
          placeholder="用户名" allowClear style={{ width: 200 }}
          onSearch={(v) => setQuery((q) => ({ ...q, username: v || undefined, page: 1 }))}
        />
        <Select
          placeholder="状态" allowClear style={{ width: 120 }}
          options={[{ value: 1, label: '启用' }, { value: 0, label: '禁用' }]}
          onChange={(v) => setQuery((q) => ({ ...q, status: v, page: 1 }))}
        />
        <Button icon={<ReloadOutlined />} onClick={() => void load(query)}>刷新</Button>
        <Button icon={<DownloadOutlined />} onClick={() => void downloadCsv('/admins/export', query, 'admins')}>
          导出
        </Button>
        <Auth code="system:admin:add">
          <Button type="primary" icon={<PlusOutlined />} onClick={openCreate}>新增</Button>
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
        title={editing ? `编辑管理员：${editing.username}` : '新增管理员'}
        open={modalOpen}
        onCancel={() => setModalOpen(false)}
        onOk={() => void submit()}
        destroyOnClose
        width={560}
      >
        <Form form={form} labelCol={{ span: 5 }} wrapperCol={{ span: 18 }}>
          {!editing && (
            <>
              <Form.Item name="username" label="用户名" rules={[{ required: true, min: 3, message: '至少 3 位' }]}>
                <Input autoComplete="off" />
              </Form.Item>
              <Form.Item name="password" label="初始密码" rules={[{ required: true, min: 6, message: '至少 6 位' }]}>
                <Input.Password autoComplete="new-password" />
              </Form.Item>
            </>
          )}
          <Form.Item name="nickname" label="昵称" rules={[{ required: true, message: '请输入昵称' }]}>
            <Input />
          </Form.Item>
          <Form.Item name="dept_id" label="部门">
            <Select
              allowClear placeholder="选择部门"
              options={[{ value: 0, label: '（无）' }, ...toDeptOptions(depts)]}
            />
          </Form.Item>
          <Form.Item
            name="role_ids" label="角色"
            extra={hasUngrantable ? '灰掉的角色超出你的数据权限，无法授予' : undefined}
          >
            <Select
              mode="multiple" allowClear placeholder="选择角色"
              // 授不出去的角色不隐藏、只禁用：让人看见它存在，也知道为什么点不动。
              // 实测：已选中的禁用项仍显示为已选（编辑别人授过宽角色的人不会丢），
              // 但标签上没有 ×，要清掉只能走选择框的 allowClear。
              options={roles.map((r) => ({ value: r.id, label: r.name, disabled: r.grantable === false }))}
            />
          </Form.Item>
          <Form.Item name="email" label="邮箱"><Input /></Form.Item>
          <Form.Item name="phone" label="手机号"><Input /></Form.Item>
          <Form.Item name="sex" label="性别">
            <Select options={[{ value: 0, label: '未知' }, { value: 1, label: '男' }, { value: 2, label: '女' }]} />
          </Form.Item>
          <Form.Item name="status" label="状态">
            <Select options={[{ value: 1, label: '启用' }, { value: 0, label: '禁用' }]} />
          </Form.Item>
          <Form.Item name="remark" label="备注"><Input.TextArea rows={2} /></Form.Item>
        </Form>
      </Modal>
    </>
  );
}
