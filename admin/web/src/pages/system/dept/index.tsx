import { useCallback, useEffect, useState } from 'react';
import {
  App, Button, Form, Input, InputNumber, Modal, Popconfirm, Select, Space, Table, Tag, TreeSelect,
  type TreeSelectProps,
} from 'antd';
import { PlusOutlined, ReloadOutlined } from '@ant-design/icons';
import type { ColumnsType } from 'antd/es/table';
import { deptApi, type DeptForm } from '../../../api/dept';
import type { Dept } from '../../../api/types';
import Auth from '../../../auth/Auth';
import { useI18n } from '../../../i18n';

type TreeNode = NonNullable<TreeSelectProps['treeData']>[number];

/** 节点自身及其子孙的 id（父级选择里禁用，避免挂到自己下面成环）。 */
function subtreeIds(nodes: Dept[], id: string): Set<string> {
  const target = (function find(list: Dept[]): Dept | undefined {
    for (const d of list) {
      if (d.id === id) return d;
      const hit = d.children && find(d.children);
      if (hit) return hit;
    }
    return undefined;
  })(nodes);
  const out = new Set<string>();
  const walk = (d: Dept) => { out.add(d.id); d.children?.forEach(walk); };
  if (target) walk(target);
  return out;
}

/** 树里所有节点的 id（编辑时判断回显的 parent_id 是不是真节点）。 */
const allIds = (nodes: Dept[]): string[] => nodes.flatMap((d) => [d.id, ...allIds(d.children ?? [])]);

/** 部门树 → TreeSelect 数据（顶级用 `''`）。 */
function toTreeData(nodes: Dept[], blocked: Set<string> = new Set()): TreeNode[] {
  return nodes.map((d) => ({
    value: d.id,
    title: d.name,
    disabled: blocked.has(d.id),
    children: d.children?.length ? toTreeData(d.children, blocked) : undefined,
  }));
}

export default function DeptPage() {
  const { message } = App.useApp();
  const { t } = useI18n();
  const [form] = Form.useForm<DeptForm>();
  const [rows, setRows] = useState<Dept[]>([]);
  const [loading, setLoading] = useState(false);
  const [modalOpen, setModalOpen] = useState(false);
  const [editing, setEditing] = useState<Dept | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      setRows(await deptApi.tree());
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { void load(); }, [load]);

  const openCreate = (parentId = '') => {
    setEditing(null);
    form.resetFields();
    form.setFieldsValue({ parent_id: parentId, sort: 0, status: 1 });
    setModalOpen(true);
  };

  const openEdit = (row: Dept) => {
    setEditing(row);
    // 根节点的 parent_id 是后端 enc(0) 的短串，不在树里 → 归一成 ''（顶级，后端 '' = 0）
    form.setFieldsValue({
      ...row,
      parent_id: allIds(rows).includes(row.parent_id) ? row.parent_id : '',
    } as unknown as DeptForm);
    setModalOpen(true);
  };

  const submit = async () => {
    const v = await form.validateFields();
    const data: DeptForm = { ...v, parent_id: v.parent_id ?? '' };
    if (editing) {
      await deptApi.update(editing.id, data);
      message.success(t('common.saved'));
    } else {
      await deptApi.create(data);
      message.success(t('common.created'));
    }
    setModalOpen(false);
    void load();
  };

  const columns: ColumnsType<Dept> = [
    { title: t('field.name'), dataIndex: 'name' },
    { title: t('field.leader'), dataIndex: 'leader', render: (v) => v || '-' },
    { title: t('field.telephone'), dataIndex: 'phone', render: (v) => v || '-' },
    { title: t('field.sort'), dataIndex: 'sort', width: 80 },
    {
      title: t('field.status'), dataIndex: 'status', width: 90,
      render: (v: number) => (
        <Tag color={v === 1 ? 'green' : 'red'}>{v === 1 ? t('common.enabled') : t('common.disabled')}</Tag>
      ),
    },
    {
      title: t('common.actions'), width: 200,
      render: (_, row) => (
        <Space>
          <Auth code="system:dept:add">
            <Button size="small" type="link" onClick={() => openCreate(row.id)}>{t('dept.add_child')}</Button>
          </Auth>
          <Auth code="system:dept:edit">
            <Button size="small" type="link" onClick={() => openEdit(row)}>{t('common.edit')}</Button>
          </Auth>
          <Auth code="system:dept:remove">
            <Popconfirm
              title={t('dept.delete_confirm')}
              onConfirm={async () => { await deptApi.remove(row.id); message.success(t('common.deleted')); void load(); }}
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
        <Button icon={<ReloadOutlined />} onClick={() => void load()}>{t('common.refresh')}</Button>
        <Auth code="system:dept:add">
          <Button type="primary" icon={<PlusOutlined />} onClick={() => openCreate()}>{t('common.add')}</Button>
        </Auth>
      </Space>

      <Table<Dept>
        rowKey="id"
        size="small"
        loading={loading}
        columns={columns}
        dataSource={rows}
        pagination={false}
        scroll={{ x: 800 }}
      />

      <Modal
        title={editing ? t('dept.edit_title', { name: editing.name }) : t('dept.create_title')}
        open={modalOpen}
        onCancel={() => setModalOpen(false)}
        onOk={() => void submit()}
        destroyOnClose
        width={520}
      >
        <Form form={form} labelCol={{ span: 5 }} wrapperCol={{ span: 18 }}>
          <Form.Item name="parent_id" label={t('field.parent_dept')}>
            <TreeSelect
              allowClear placeholder={t('common.top')} treeDefaultExpandAll
              treeData={[{
                value: '',
                title: t('common.top'),
                children: toTreeData(rows, editing ? subtreeIds(rows, editing.id) : undefined),
              }]}
            />
          </Form.Item>
          <Form.Item
            name="name" label={t('field.name')}
            rules={[{ required: true, message: t('validate.required', { field: t('field.name') }) }]}
          >
            <Input />
          </Form.Item>
          <Form.Item name="leader" label={t('field.leader')}><Input /></Form.Item>
          <Form.Item name="phone" label={t('field.telephone')}><Input /></Form.Item>
          <Form.Item name="sort" label={t('field.sort')}><InputNumber min={0} /></Form.Item>
          <Form.Item name="status" label={t('field.status')}>
            <Select options={[{ value: 1, label: t('common.enabled') }, { value: 0, label: t('common.disabled') }]} />
          </Form.Item>
        </Form>
      </Modal>
    </>
  );
}
