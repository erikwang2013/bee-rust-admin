import { useCallback, useEffect, useState } from 'react';
import {
  App, Button, Divider, Drawer, Form, Input, InputNumber, Modal, Popconfirm, Select, Space, Table, Tag, Tree,
} from 'antd';
import type { DataNode } from 'antd/es/tree';
import { PlusOutlined, ReloadOutlined } from '@ant-design/icons';
import type { ColumnsType } from 'antd/es/table';
import { roleApi, type RoleForm, type RoleQuery } from '../../../api/role';
import { menuApi } from '../../../api/menu';
import { deptApi } from '../../../api/dept';
import { DATA_SCOPE_LABELS, type Dept, type Menu, type Role } from '../../../api/types';
import Auth from '../../../auth/Auth';

/** 菜单树 → antd Tree；按钮类型标注出来便于区分。 */
const toMenuTree = (nodes: Menu[]): DataNode[] =>
  nodes.map((n) => ({
    key: n.id,
    title: `${n.name}${n.type === 'F' ? '（按钮）' : ''}`,
    children: n.children ? toMenuTree(n.children) : undefined,
  }));

const toDeptTree = (nodes: Dept[]): DataNode[] =>
  nodes.map((d) => ({ key: d.id, title: d.name, children: d.children ? toDeptTree(d.children) : undefined }));

const SCOPE_OPTIONS = Object.entries(DATA_SCOPE_LABELS).map(([value, label]) => ({
  value: Number(value),
  label,
}));

export default function RolePage() {
  const { message } = App.useApp();
  const [form] = Form.useForm<RoleForm>();
  const [query, setQuery] = useState<RoleQuery>({ page: 1, size: 10 });
  const [rows, setRows] = useState<Role[]>([]);
  const [total, setTotal] = useState(0);
  const [loading, setLoading] = useState(false);
  const [modalOpen, setModalOpen] = useState(false);
  const [editing, setEditing] = useState<Role | null>(null);

  const [treeData, setTreeData] = useState<Menu[]>([]);
  const [deptTree, setDeptTree] = useState<Dept[]>([]);
  const [permOpen, setPermOpen] = useState(false);
  const [permRole, setPermRole] = useState<Role | null>(null);
  const [checked, setChecked] = useState<number[]>([]);
  const [deptChecked, setDeptChecked] = useState<number[]>([]);

  const load = useCallback(async (q: RoleQuery) => {
    setLoading(true);
    try {
      const res = await roleApi.list(q);
      setRows(res.list);
      setTotal(res.total);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { void load(query); }, [query, load]);

  const openCreate = () => {
    setEditing(null);
    form.resetFields();
    form.setFieldsValue({ sort: 0, data_scope: 1, status: 1 } as unknown as RoleForm);
    setModalOpen(true);
  };

  const openEdit = (row: Role) => {
    setEditing(row);
    form.setFieldsValue(row as unknown as RoleForm);
    setModalOpen(true);
  };

  const submit = async () => {
    const v = await form.validateFields();
    if (editing) {
      await roleApi.update(editing.id, v);
      message.success('已保存');
    } else {
      await roleApi.create(v);
      message.success('已创建');
    }
    setModalOpen(false);
    void load(query);
  };

  const openPerm = async (row: Role) => {
    setPermRole(row);
    const [tree, ids, deptIds, dtree] = await Promise.all([
      menuApi.tree(), roleApi.menus(row.id), roleApi.depts(row.id), deptApi.tree(),
    ]);
    setTreeData(tree);
    setChecked(ids);
    setDeptChecked(deptIds);
    setDeptTree(dtree);
    setPermOpen(true);
  };

  const savePerm = async () => {
    if (!permRole) return;
    await roleApi.setMenus(permRole.id, checked);
    if (permRole.data_scope === 5) await roleApi.setDepts(permRole.id, deptChecked);
    message.success('权限已保存');
    setPermOpen(false);
  };

  const columns: ColumnsType<Role> = [
    { title: 'ID', dataIndex: 'id', width: 70 },
    { title: '角色名', dataIndex: 'name' },
    { title: '角色标识', dataIndex: 'code' },
    { title: '排序', dataIndex: 'sort', width: 80 },
    { title: '数据范围', dataIndex: 'data_scope', render: (v: number) => DATA_SCOPE_LABELS[v] ?? v },
    {
      title: '状态', dataIndex: 'status', width: 90,
      render: (v: number) => <Tag color={v === 1 ? 'green' : 'red'}>{v === 1 ? '启用' : '禁用'}</Tag>,
    },
    { title: '创建时间', dataIndex: 'created_at', width: 170 },
    {
      title: '操作', width: 200, fixed: 'right',
      render: (_, row) => (
        <Space>
          <Auth code="system:role:edit">
            <Button size="small" type="link" onClick={() => openEdit(row)}>编辑</Button>
          </Auth>
          <Auth code="system:role:edit">
            <Button size="small" type="link" onClick={() => void openPerm(row)}>权限</Button>
          </Auth>
          <Auth code="system:role:remove">
            <Popconfirm
              title="确认删除该角色？"
              onConfirm={async () => { await roleApi.remove(row.id); message.success('已删除'); void load(query); }}
            >
              <Button size="small" type="link" danger>删除</Button>
            </Popconfirm>
          </Auth>
        </Space>
      ),
    },
  ];

  return (
    <>
      <Space style={{ marginBottom: 16 }} wrap>
        <Input.Search
          placeholder="角色名" allowClear style={{ width: 200 }}
          onSearch={(v) => setQuery((q) => ({ ...q, name: v || undefined, page: 1 }))}
        />
        <Select
          placeholder="状态" allowClear style={{ width: 120 }}
          options={[{ value: 1, label: '启用' }, { value: 0, label: '禁用' }]}
          onChange={(v) => setQuery((q) => ({ ...q, status: v, page: 1 }))}
        />
        <Button icon={<ReloadOutlined />} onClick={() => void load(query)}>刷新</Button>
        <Auth code="system:role:add">
          <Button type="primary" icon={<PlusOutlined />} onClick={openCreate}>新增</Button>
        </Auth>
      </Space>

      <Table<Role>
        rowKey="id"
        size="small"
        loading={loading}
        columns={columns}
        dataSource={rows}
        scroll={{ x: 1000 }}
        pagination={{
          current: query.page, pageSize: query.size, total, showSizeChanger: true,
          onChange: (page, size) => setQuery((q) => ({ ...q, page, size })),
        }}
      />

      <Modal
        title={editing ? `编辑角色：${editing.name}` : '新增角色'}
        open={modalOpen}
        onCancel={() => setModalOpen(false)}
        onOk={() => void submit()}
        destroyOnClose
        width={520}
      >
        <Form form={form} labelCol={{ span: 5 }} wrapperCol={{ span: 18 }}>
          <Form.Item name="name" label="角色名" rules={[{ required: true, message: '请输入角色名' }]}>
            <Input />
          </Form.Item>
          <Form.Item name="code" label="角色标识" rules={[{ required: true, message: '请输入角色标识' }]}>
            <Input disabled={!!editing} placeholder="如 admin" />
          </Form.Item>
          <Form.Item name="sort" label="排序"><InputNumber min={0} /></Form.Item>
          <Form.Item name="data_scope" label="数据范围">
            <Select options={SCOPE_OPTIONS} />
          </Form.Item>
          <Form.Item name="status" label="状态">
            <Select options={[{ value: 1, label: '启用' }, { value: 0, label: '禁用' }]} />
          </Form.Item>
          <Form.Item name="remark" label="备注"><Input.TextArea rows={2} /></Form.Item>
        </Form>
      </Modal>

      <Drawer
        title={`分配权限：${permRole?.name ?? ''}`}
        width={420}
        open={permOpen}
        onClose={() => setPermOpen(false)}
        extra={<Button type="primary" onClick={() => void savePerm()}>保存</Button>}
      >
        <Tree
          checkable
          defaultExpandAll
          treeData={toMenuTree(treeData)}
          checkedKeys={checked}
          onCheck={(keys) => setChecked((Array.isArray(keys) ? keys : keys.checked) as number[])}
        />
        {permRole?.data_scope === 5 && (
          <>
            <Divider orientation="left">数据权限</Divider>
            <Tree
              checkable
              defaultExpandAll
              treeData={toDeptTree(deptTree)}
              checkedKeys={deptChecked}
              onCheck={(keys) => setDeptChecked((Array.isArray(keys) ? keys : keys.checked) as number[])}
            />
          </>
        )}
      </Drawer>
    </>
  );
}
