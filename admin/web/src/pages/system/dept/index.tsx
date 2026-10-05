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

type TreeNode = NonNullable<TreeSelectProps['treeData']>[number];

/** 节点自身及其子孙的 id（父级选择里禁用，避免挂到自己下面成环）。 */
function subtreeIds(nodes: Dept[], id: number): Set<number> {
  const target = (function find(list: Dept[]): Dept | undefined {
    for (const d of list) {
      if (d.id === id) return d;
      const hit = d.children && find(d.children);
      if (hit) return hit;
    }
    return undefined;
  })(nodes);
  const out = new Set<number>();
  const walk = (d: Dept) => { out.add(d.id); d.children?.forEach(walk); };
  if (target) walk(target);
  return out;
}

/** 部门树 → TreeSelect 数据（顶级用 id=0）。 */
function toTreeData(nodes: Dept[], blocked: Set<number> = new Set()): TreeNode[] {
  return nodes.map((d) => ({
    value: d.id,
    title: d.name,
    disabled: blocked.has(d.id),
    children: d.children?.length ? toTreeData(d.children, blocked) : undefined,
  }));
}

export default function DeptPage() {
  const { message } = App.useApp();
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

  const openCreate = (parentId = 0) => {
    setEditing(null);
    form.resetFields();
    form.setFieldsValue({ parent_id: parentId, sort: 0, status: 1 });
    setModalOpen(true);
  };

  const openEdit = (row: Dept) => {
    setEditing(row);
    form.setFieldsValue(row as unknown as DeptForm);
    setModalOpen(true);
  };

  const submit = async () => {
    const v = await form.validateFields();
    const data: DeptForm = { ...v, parent_id: v.parent_id ?? 0 };
    if (editing) {
      await deptApi.update(editing.id, data);
      message.success('已保存');
    } else {
      await deptApi.create(data);
      message.success('已创建');
    }
    setModalOpen(false);
    void load();
  };

  const columns: ColumnsType<Dept> = [
    { title: '名称', dataIndex: 'name' },
    { title: '负责人', dataIndex: 'leader', render: (v) => v || '-' },
    { title: '电话', dataIndex: 'phone', render: (v) => v || '-' },
    { title: '排序', dataIndex: 'sort', width: 80 },
    {
      title: '状态', dataIndex: 'status', width: 90,
      render: (v: number) => <Tag color={v === 1 ? 'green' : 'red'}>{v === 1 ? '启用' : '禁用'}</Tag>,
    },
    {
      title: '操作', width: 200,
      render: (_, row) => (
        <Space>
          <Auth code="system:dept:add">
            <Button size="small" type="link" onClick={() => openCreate(row.id)}>新增子部门</Button>
          </Auth>
          <Auth code="system:dept:edit">
            <Button size="small" type="link" onClick={() => openEdit(row)}>编辑</Button>
          </Auth>
          <Auth code="system:dept:remove">
            <Popconfirm
              title="确认删除该部门？"
              onConfirm={async () => { await deptApi.remove(row.id); message.success('已删除'); void load(); }}
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
        <Button icon={<ReloadOutlined />} onClick={() => void load()}>刷新</Button>
        <Auth code="system:dept:add">
          <Button type="primary" icon={<PlusOutlined />} onClick={() => openCreate(0)}>新增</Button>
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
        title={editing ? `编辑部门：${editing.name}` : '新增部门'}
        open={modalOpen}
        onCancel={() => setModalOpen(false)}
        onOk={() => void submit()}
        destroyOnClose
        width={520}
      >
        <Form form={form} labelCol={{ span: 5 }} wrapperCol={{ span: 18 }}>
          <Form.Item name="parent_id" label="上级部门">
            <TreeSelect
              allowClear placeholder="顶级" treeDefaultExpandAll
              treeData={[{ value: 0, title: '顶级', children: toTreeData(rows, editing ? subtreeIds(rows, editing.id) : undefined) }]}
            />
          </Form.Item>
          <Form.Item name="name" label="名称" rules={[{ required: true, message: '请输入名称' }]}>
            <Input />
          </Form.Item>
          <Form.Item name="leader" label="负责人"><Input /></Form.Item>
          <Form.Item name="phone" label="电话"><Input /></Form.Item>
          <Form.Item name="sort" label="排序"><InputNumber min={0} /></Form.Item>
          <Form.Item name="status" label="状态">
            <Select options={[{ value: 1, label: '启用' }, { value: 0, label: '禁用' }]} />
          </Form.Item>
        </Form>
      </Modal>
    </>
  );
}
