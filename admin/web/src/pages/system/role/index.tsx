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
import type { Dept, Menu, Role } from '../../../api/types';
import Auth from '../../../auth/Auth';
import { useI18n, type I18nKey, type TFunc } from '../../../i18n';

/** 数据范围取值 1-5 → 词表键（标签在词表里，业务数据（角色名/菜单名）才来自库）。 */
const SCOPE_KEYS: Record<number, I18nKey> = {
  1: 'scope.all',
  2: 'scope.dept_below',
  3: 'scope.dept',
  4: 'scope.self',
  5: 'scope.custom',
};

/** 菜单树 → antd Tree；按钮类型标注出来便于区分。 */
const toMenuTree = (nodes: Menu[], t: TFunc): DataNode[] =>
  nodes.map((n) => ({
    key: n.id,
    title: `${n.name}${n.type === 'F' ? t('role.button_tag') : ''}`,
    children: n.children ? toMenuTree(n.children, t) : undefined,
  }));

const toDeptTree = (nodes: Dept[]): DataNode[] =>
  nodes.map((d) => ({ key: d.id, title: d.name, children: d.children ? toDeptTree(d.children) : undefined }));

export default function RolePage() {
  const { message } = App.useApp();
  const { t } = useI18n();
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
      message.success(t('common.saved'));
    } else {
      await roleApi.create(v);
      message.success(t('common.created'));
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
    message.success(t('role.perm_saved'));
    setPermOpen(false);
  };

  const scopeText = (v: number) => (SCOPE_KEYS[v] ? t(SCOPE_KEYS[v]) : String(v));

  const columns: ColumnsType<Role> = [
    { title: t('field.id'), dataIndex: 'id', width: 70 },
    { title: t('field.role_name'), dataIndex: 'name' },
    { title: t('field.role_code'), dataIndex: 'code' },
    { title: t('field.sort'), dataIndex: 'sort', width: 80 },
    { title: t('field.data_scope'), dataIndex: 'data_scope', render: scopeText },
    {
      title: t('field.status'), dataIndex: 'status', width: 90,
      render: (v: number) => (
        <Tag color={v === 1 ? 'green' : 'red'}>{v === 1 ? t('common.enabled') : t('common.disabled')}</Tag>
      ),
    },
    { title: t('field.created_at'), dataIndex: 'created_at', width: 170 },
    {
      title: t('common.actions'), width: 200, fixed: 'right',
      render: (_, row) => (
        <Space>
          <Auth code="system:role:edit">
            <Button size="small" type="link" onClick={() => openEdit(row)}>{t('common.edit')}</Button>
          </Auth>
          <Auth code="system:role:edit">
            <Button size="small" type="link" onClick={() => void openPerm(row)}>{t('role.perm_button')}</Button>
          </Auth>
          <Auth code="system:role:remove">
            <Popconfirm
              title={t('role.delete_confirm')}
              onConfirm={async () => { await roleApi.remove(row.id); message.success(t('common.deleted')); void load(query); }}
            >
              <Button size="small" type="link" danger>{t('common.delete')}</Button>
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
          placeholder={t('field.role_name')} allowClear style={{ width: 200 }}
          onSearch={(v) => setQuery((q) => ({ ...q, name: v || undefined, page: 1 }))}
        />
        <Select
          placeholder={t('field.status')} allowClear style={{ width: 120 }}
          options={[{ value: 1, label: t('common.enabled') }, { value: 0, label: t('common.disabled') }]}
          onChange={(v) => setQuery((q) => ({ ...q, status: v, page: 1 }))}
        />
        <Button icon={<ReloadOutlined />} onClick={() => void load(query)}>{t('common.refresh')}</Button>
        <Auth code="system:role:add">
          <Button type="primary" icon={<PlusOutlined />} onClick={openCreate}>{t('common.add')}</Button>
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
        title={editing ? t('role.edit_title', { name: editing.name }) : t('role.create_title')}
        open={modalOpen}
        onCancel={() => setModalOpen(false)}
        onOk={() => void submit()}
        destroyOnClose
        width={520}
      >
        <Form form={form} labelCol={{ span: 5 }} wrapperCol={{ span: 18 }}>
          <Form.Item
            name="name" label={t('field.role_name')}
            rules={[{ required: true, message: t('validate.required', { field: t('field.role_name') }) }]}
          >
            <Input />
          </Form.Item>
          <Form.Item
            name="code" label={t('field.role_code')}
            rules={[{ required: true, message: t('validate.required', { field: t('field.role_code') }) }]}
          >
            <Input disabled={!!editing} placeholder={t('role.code_example')} />
          </Form.Item>
          <Form.Item name="sort" label={t('field.sort')}><InputNumber min={0} /></Form.Item>
          <Form.Item name="data_scope" label={t('field.data_scope')}>
            <Select options={Object.entries(SCOPE_KEYS).map(([v, k]) => ({ value: Number(v), label: t(k) }))} />
          </Form.Item>
          <Form.Item name="status" label={t('field.status')}>
            <Select options={[{ value: 1, label: t('common.enabled') }, { value: 0, label: t('common.disabled') }]} />
          </Form.Item>
          <Form.Item name="remark" label={t('field.remark')}><Input.TextArea rows={2} /></Form.Item>
        </Form>
      </Modal>

      <Drawer
        title={t('role.perm_title', { name: permRole?.name ?? '' })}
        width={420}
        open={permOpen}
        onClose={() => setPermOpen(false)}
        extra={<Button type="primary" onClick={() => void savePerm()}>{t('common.save')}</Button>}
      >
        <Tree
          checkable
          defaultExpandAll
          treeData={toMenuTree(treeData, t)}
          checkedKeys={checked}
          onCheck={(keys) => setChecked((Array.isArray(keys) ? keys : keys.checked) as number[])}
        />
        {permRole?.data_scope === 5 && (
          <>
            <Divider orientation="left">{t('role.data_perm')}</Divider>
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
