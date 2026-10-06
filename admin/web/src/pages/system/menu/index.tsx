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
import { useI18n, type I18nKey } from '../../../i18n';

const TYPE_KEYS: Record<Menu['type'], I18nKey> = {
  M: 'menu.type_dir', C: 'menu.type_menu', F: 'menu.type_button',
};
const TYPE_COLORS: Record<Menu['type'], string> = { M: 'blue', C: 'green', F: 'orange' };

type TreeNode = NonNullable<TreeSelectProps['treeData']>[number];

/** 节点自身及其子孙的 id（父级选择里禁用，避免挂到自己下面成环）。 */
function subtreeIds(nodes: Menu[], id: number): Set<number> {
  const target = (function find(list: Menu[]): Menu | undefined {
    for (const n of list) {
      if (n.id === id) return n;
      const hit = n.children && find(n.children);
      if (hit) return hit;
    }
    return undefined;
  })(nodes);
  const out = new Set<number>();
  const walk = (n: Menu) => { out.add(n.id); n.children?.forEach(walk); };
  if (target) walk(target);
  return out;
}

/** 菜单树 → TreeSelect 数据（顶级用 id=0）。 */
function toTreeData(nodes: Menu[], blocked: Set<number> = new Set()): TreeNode[] {
  return nodes.map((n) => ({
    value: n.id,
    title: n.name,
    disabled: blocked.has(n.id),
    children: n.children?.length ? toTreeData(n.children, blocked) : undefined,
  }));
}

export default function MenuPage() {
  const { message } = App.useApp();
  const { t } = useI18n();
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
      message.success(t('common.saved'));
    } else {
      await menuApi.create(data);
      message.success(t('common.created'));
    }
    setModalOpen(false);
    void load();
  };

  const columns: ColumnsType<Menu> = [
    { title: t('field.name'), dataIndex: 'name' },
    {
      title: t('field.type'), dataIndex: 'type', width: 90,
      render: (v: Menu['type']) => <Tag color={TYPE_COLORS[v]}>{t(TYPE_KEYS[v])}</Tag>,
    },
    { title: t('field.perm'), dataIndex: 'perm' },
    { title: t('field.path'), dataIndex: 'path' },
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
          <Auth code="system:menu:add">
            <Button size="small" type="link" onClick={() => openCreate(row.id)}>{t('menu.add_child')}</Button>
          </Auth>
          <Auth code="system:menu:edit">
            <Button size="small" type="link" onClick={() => openEdit(row)}>{t('common.edit')}</Button>
          </Auth>
          <Auth code="system:menu:remove">
            <Popconfirm
              title={t('menu.delete_confirm')}
              onConfirm={async () => { await menuApi.remove(row.id); message.success(t('common.deleted')); void load(); }}
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
        <Auth code="system:menu:add">
          <Button type="primary" icon={<PlusOutlined />} onClick={() => openCreate(0)}>{t('common.add')}</Button>
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
        title={editing ? t('menu.edit_title', { name: editing.name }) : t('menu.create_title')}
        open={modalOpen}
        onCancel={() => setModalOpen(false)}
        onOk={() => void submit()}
        destroyOnClose
        width={560}
      >
        <Form form={form} labelCol={{ span: 5 }} wrapperCol={{ span: 18 }}>
          <Form.Item name="parent_id" label={t('field.parent_menu')}>
            <TreeSelect
              allowClear placeholder={t('common.top')} treeDefaultExpandAll
              treeData={[{
                value: 0,
                title: t('common.top'),
                children: toTreeData(rows, editing ? subtreeIds(rows, editing.id) : undefined),
              }]}
            />
          </Form.Item>
          <Form.Item name="type" label={t('field.type')}>
            <Radio.Group
              options={(['M', 'C', 'F'] as const).map((v) => ({ value: v, label: t(TYPE_KEYS[v]) }))}
            />
          </Form.Item>
          <Form.Item
            name="name" label={t('field.name')}
            rules={[{ required: true, message: t('validate.required', { field: t('field.name') }) }]}
          >
            <Input />
          </Form.Item>
          {(type === 'M' || type === 'C') && (
            <Form.Item
              name="path" label={t('field.route_path')}
              rules={[{ required: true, message: t('validate.required', { field: t('field.route_path') }) }]}
            >
              <Input placeholder={t('menu.path_example')} />
            </Form.Item>
          )}
          {type === 'C' && (
            <Form.Item name="component" label={t('field.component')}>
              <Input placeholder={t('menu.component_hint')} />
            </Form.Item>
          )}
          {(type === 'C' || type === 'F') && (
            <Form.Item
              name="perm"
              label={t('field.perm')}
              rules={type === 'F'
                ? [{ required: true, message: t('validate.required', { field: t('field.perm') }) }]
                : []}
            >
              <Input placeholder={t('menu.perm_example')} />
            </Form.Item>
          )}
          {(type === 'M' || type === 'C') && (
            <Form.Item name="icon" label={t('field.icon')}>
              <Input placeholder={t('menu.icon_example')} />
            </Form.Item>
          )}
          <Form.Item name="sort" label={t('field.sort')}><InputNumber min={0} /></Form.Item>
          {(type === 'M' || type === 'C') && (
            <Form.Item name="visible" label={t('field.visible')}>
              <Select options={[{ value: 1, label: t('menu.show') }, { value: 0, label: t('menu.hide') }]} />
            </Form.Item>
          )}
          <Form.Item name="status" label={t('field.status')}>
            <Select options={[{ value: 1, label: t('common.enabled') }, { value: 0, label: t('common.disabled') }]} />
          </Form.Item>
        </Form>
      </Modal>
    </>
  );
}
