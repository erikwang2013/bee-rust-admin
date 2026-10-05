import { useCallback, useEffect, useState } from 'react';
import {
  App, Button, Form, Input, InputNumber, Modal, Popconfirm, Radio, Select, Space, Table, Tag, TreeSelect,
  type TreeSelectProps,
} from 'antd';
import { PlusOutlined, ReloadOutlined } from '@ant-design/icons';
import type { ColumnsType } from 'antd/es/table';
import { menuApi, type MenuForm } from '../../../api/menu';
import type { Menu } from '../../../api/types';
import Auth from '../../../auth/Auth';

const TYPE_LABELS: Record<Menu['type'], string> = { M: '目录', C: '菜单', F: '按钮' };
const TYPE_COLORS: Record<Menu['type'], string> = { M: 'blue', C: 'green', F: 'orange' };

type TreeNode = NonNullable<TreeSelectProps['treeData']>[number];

/** 菜单树 → TreeSelect 数据（顶级用 id=0）。 */
function toTreeData(nodes: Menu[]): TreeNode[] {
  return nodes.map((n) => ({
    value: n.id,
    title: n.name,
    children: n.children?.length ? toTreeData(n.children) : undefined,
  }));
}

export default function MenuPage() {
  const { message } = App.useApp();
  const [form] = Form.useForm<MenuForm>();
  const [rows, setRows] = useState<Menu[]>([]);
  const [loading, setLoading] = useState(false);
  const [modalOpen, setModalOpen] = useState(false);
  const [editing, setEditing] = useState<Menu | null>(null);
  const type = Form.useWatch('type', form) ?? 'C';

  const load = useCallback(async () => {
    setLoading(true);
    try {
      setRows(await menuApi.tree());
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { void load(); }, [load]);

  const openCreate = (parentId = 0) => {
    setEditing(null);
    form.resetFields();
    form.setFieldsValue({ parent_id: parentId, type: 'C', sort: 0, visible: 1, status: 1 });
    setModalOpen(true);
  };

  const openEdit = (row: Menu) => {
    setEditing(row);
    form.setFieldsValue(row as unknown as MenuForm);
    setModalOpen(true);
  };

  const submit = async () => {
    const v = await form.validateFields();
    const data: MenuForm = {
      ...v,
      parent_id: v.parent_id ?? 0,
      // 按类型归一化：切类型后残留的字段清空，避免脏值入库
      ...(v.type === 'M' ? { component: '', perm: '' } : {}),
      ...(v.type === 'F' ? { path: '', component: '', icon: '' } : {}),
    };
    if (editing) {
      await menuApi.update(editing.id, data);
      message.success('已保存');
    } else {
      await menuApi.create(data);
      message.success('已创建');
    }
    setModalOpen(false);
    void load();
  };

  const columns: ColumnsType<Menu> = [
    { title: '名称', dataIndex: 'name' },
    {
      title: '类型', dataIndex: 'type', width: 90,
      render: (v: Menu['type']) => <Tag color={TYPE_COLORS[v]}>{TYPE_LABELS[v]}</Tag>,
    },
    { title: '权限码', dataIndex: 'perm' },
    { title: '路径', dataIndex: 'path' },
    { title: '排序', dataIndex: 'sort', width: 80 },
    {
      title: '状态', dataIndex: 'status', width: 90,
      render: (v: number) => <Tag color={v === 1 ? 'green' : 'red'}>{v === 1 ? '启用' : '禁用'}</Tag>,
    },
    {
      title: '操作', width: 200,
      render: (_, row) => (
        <Space>
          <Auth code="system:menu:add">
            <Button size="small" type="link" onClick={() => openCreate(row.id)}>新增子项</Button>
          </Auth>
          <Auth code="system:menu:edit">
            <Button size="small" type="link" onClick={() => openEdit(row)}>编辑</Button>
          </Auth>
          <Auth code="system:menu:remove">
            <Popconfirm
              title="确认删除该菜单？"
              onConfirm={async () => { await menuApi.remove(row.id); message.success('已删除'); void load(); }}
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
        <Auth code="system:menu:add">
          <Button type="primary" icon={<PlusOutlined />} onClick={() => openCreate(0)}>新增</Button>
        </Auth>
      </Space>

      <Table<Menu>
        rowKey="id"
        size="small"
        loading={loading}
        columns={columns}
        dataSource={rows}
        pagination={false}
        scroll={{ x: 900 }}
      />

      <Modal
        title={editing ? `编辑菜单：${editing.name}` : '新增菜单'}
        open={modalOpen}
        onCancel={() => setModalOpen(false)}
        onOk={() => void submit()}
        destroyOnClose
        width={560}
      >
        <Form form={form} labelCol={{ span: 5 }} wrapperCol={{ span: 18 }}>
          <Form.Item name="parent_id" label="上级菜单">
            <TreeSelect
              allowClear placeholder="顶级" treeDefaultExpandAll
              treeData={[{ value: 0, title: '顶级', children: toTreeData(rows) }]}
            />
          </Form.Item>
          <Form.Item name="type" label="类型">
            <Radio.Group
              options={[
                { value: 'M', label: '目录' },
                { value: 'C', label: '菜单' },
                { value: 'F', label: '按钮' },
              ]}
            />
          </Form.Item>
          <Form.Item name="name" label="名称" rules={[{ required: true, message: '请输入名称' }]}>
            <Input />
          </Form.Item>
          {(type === 'M' || type === 'C') && (
            <Form.Item name="path" label="路由路径" rules={[{ required: true, message: '请输入路由路径' }]}>
              <Input placeholder="如 /system/admin" />
            </Form.Item>
          )}
          {type === 'C' && (
            <Form.Item name="component" label="组件">
              <Input placeholder="前端组件路径，可留空" />
            </Form.Item>
          )}
          {(type === 'C' || type === 'F') && (
            <Form.Item
              name="perm"
              label="权限码"
              rules={type === 'F' ? [{ required: true, message: '请输入权限码' }] : []}
            >
              <Input placeholder="如 system:admin:add" />
            </Form.Item>
          )}
          {(type === 'M' || type === 'C') && (
            <Form.Item name="icon" label="图标"><Input placeholder="如 SettingOutlined" /></Form.Item>
          )}
          <Form.Item name="sort" label="排序"><InputNumber min={0} /></Form.Item>
          {(type === 'M' || type === 'C') && (
            <Form.Item name="visible" label="显示">
              <Select options={[{ value: 1, label: '显示' }, { value: 0, label: '隐藏' }]} />
            </Form.Item>
          )}
          <Form.Item name="status" label="状态">
            <Select options={[{ value: 1, label: '启用' }, { value: 0, label: '禁用' }]} />
          </Form.Item>
        </Form>
      </Modal>
    </>
  );
}
