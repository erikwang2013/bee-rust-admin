import { useCallback, useEffect, useState } from 'react';
import {
  App, Button, Col, Form, Input, InputNumber, Modal, Popconfirm, Row, Select, Space, Table, Tag, Typography,
} from 'antd';
import { DownloadOutlined, PlusOutlined, ReloadOutlined } from '@ant-design/icons';
import type { ColumnsType } from 'antd/es/table';
import {
  dictApi,
  type DictItemForm, type DictItemQuery, type DictTypeForm, type DictTypeQuery,
} from '../../../api/dict';
import type { DictItem, DictType } from '../../../api/types';
import { dictCache } from '../../../hooks/useDict';
import Auth from '../../../auth/Auth';

const STATUS_OPTIONS = [{ value: 1, label: '启用' }, { value: 0, label: '禁用' }];

const statusTag = (v: number) => <Tag color={v === 1 ? 'green' : 'red'}>{v === 1 ? '启用' : '禁用'}</Tag>;

export default function DictPage() {
  const { message } = App.useApp();
  const [typeForm] = Form.useForm<DictTypeForm>();
  const [itemForm] = Form.useForm<DictItemForm>();

  // 左栏：字典类型
  const [typeQuery, setTypeQuery] = useState<DictTypeQuery>({ page: 1, size: 10 });
  const [types, setTypes] = useState<DictType[]>([]);
  const [typeTotal, setTypeTotal] = useState(0);
  const [typeLoading, setTypeLoading] = useState(false);
  const [selected, setSelected] = useState<DictType | null>(null);
  const [typeModal, setTypeModal] = useState(false);
  const [editingType, setEditingType] = useState<DictType | null>(null);

  // 右栏：选中类型的字典项
  const [itemQuery, setItemQuery] = useState<DictItemQuery>({ page: 1, size: 10 });
  const [items, setItems] = useState<DictItem[]>([]);
  const [itemTotal, setItemTotal] = useState(0);
  const [itemLoading, setItemLoading] = useState(false);
  const [itemModal, setItemModal] = useState(false);
  const [editingItem, setEditingItem] = useState<DictItem | null>(null);

  const loadTypes = useCallback(async (q: DictTypeQuery) => {
    setTypeLoading(true);
    try {
      const res = await dictApi.list(q);
      setTypes(res.list);
      setTypeTotal(res.total);
    } finally {
      setTypeLoading(false);
    }
  }, []);

  useEffect(() => { void loadTypes(typeQuery); }, [typeQuery, loadTypes]);

  const loadItems = useCallback(async (q: DictItemQuery) => {
    if (!q.type_code) return; // 没选类型时不请求，右栏显示空状态
    setItemLoading(true);
    try {
      const res = await dictApi.itemList(q);
      setItems(res.list);
      setItemTotal(res.total);
    } finally {
      setItemLoading(false);
    }
  }, []);

  useEffect(() => { void loadItems(itemQuery); }, [itemQuery, loadItems]);

  /** 选中类型：换类型时右栏筛选条件一起清掉，否则旧 label 会把新类型的项筛空 */
  const pick = (row: DictType) => {
    setSelected(row);
    setItemQuery((q) => ({ page: 1, size: q.size, type_code: row.code }));
  };

  const openCreateType = () => {
    setEditingType(null);
    typeForm.resetFields();
    typeForm.setFieldsValue({ status: 1 });
    setTypeModal(true);
  };

  const openEditType = (row: DictType) => {
    setEditingType(row);
    typeForm.setFieldsValue({ name: row.name, code: row.code, status: row.status, remark: row.remark });
    setTypeModal(true);
  };

  const submitType = async () => {
    const v = await typeForm.validateFields();
    if (editingType) {
      const rest = { name: v.name, status: v.status, remark: v.remark };
      await dictApi.update(editingType.id, rest);
      message.success('已保存');
      setSelected((s) => (s && s.id === editingType.id ? { ...s, ...rest } : s));
    } else {
      await dictApi.create(v);
      message.success('已创建');
    }
    setTypeModal(false);
    void loadTypes(typeQuery);
  };

  const removeType = async (row: DictType) => {
    await dictApi.remove(row.id);
    message.success('已删除');
    if (selected?.id === row.id) {
      setSelected(null);
      setItemQuery((q) => ({ page: 1, size: q.size, type_code: undefined }));
      setItems([]);
      setItemTotal(0);
    }
    void loadTypes(typeQuery);
  };

  const openCreateItem = () => {
    if (!selected) return;
    setEditingItem(null);
    itemForm.resetFields();
    itemForm.setFieldsValue({ type_code: selected.code, sort: 0, status: 1 });
    setItemModal(true);
  };

  const openEditItem = (row: DictItem) => {
    setEditingItem(row);
    itemForm.setFieldsValue(row as unknown as DictItemForm);
    setItemModal(true);
  };

  const submitItem = async () => {
    const v = await itemForm.validateFields();
    if (editingItem) {
      await dictApi.itemUpdate(editingItem.id, v);
      message.success('已保存');
    } else {
      await dictApi.itemCreate(v);
      message.success('已创建');
    }
    dictCache.delete(v.type_code); // 别处下拉若已缓存该 code，丢弃旧选项
    setItemModal(false);
    void loadItems(itemQuery);
  };

  const typeColumns: ColumnsType<DictType> = [
    { title: '名称', dataIndex: 'name' },
    { title: '编码', dataIndex: 'code' },
    { title: '状态', dataIndex: 'status', width: 80, render: statusTag },
    { title: '备注', dataIndex: 'remark', ellipsis: true },
    {
      title: '操作', width: 110, fixed: 'right',
      render: (_, row) => (
        <Space>
          <Auth code="system:dict:edit">
            <Button size="small" type="link" onClick={() => openEditType(row)}>编辑</Button>
          </Auth>
          <Auth code="system:dict:remove">
            <Popconfirm
              title={`确认删除字典类型「${row.name}」？`}
              description="该类型下的所有字典项会一并删除。"
              okText="删除"
              okButtonProps={{ danger: true }}
              onConfirm={() => void removeType(row)}
            >
              <Button size="small" type="link" danger>删除</Button>
            </Popconfirm>
          </Auth>
        </Space>
      ),
    },
  ];

  const itemColumns: ColumnsType<DictItem> = [
    { title: '标签', dataIndex: 'label' },
    { title: '值', dataIndex: 'value' },
    { title: '排序', dataIndex: 'sort', width: 70 },
    { title: '状态', dataIndex: 'status', width: 80, render: statusTag },
    { title: '备注', dataIndex: 'remark', ellipsis: true },
    {
      title: '操作', width: 110, fixed: 'right',
      render: (_, row) => (
        <Space>
          <Auth code="system:dict:edit">
            <Button size="small" type="link" onClick={() => openEditItem(row)}>编辑</Button>
          </Auth>
          <Auth code="system:dict:remove">
            <Popconfirm
              title="确认删除该字典项？"
              okText="删除"
              okButtonProps={{ danger: true }}
              onConfirm={async () => {
                await dictApi.itemRemove(row.id);
                message.success('已删除');
                dictCache.delete(row.type_code);
                void loadItems(itemQuery);
              }}
            >
              <Button size="small" type="link" danger>删除</Button>
            </Popconfirm>
          </Auth>
        </Space>
      ),
    },
  ];

  return (
    <Row gutter={16}>
      <Col xs={24} xl={10}>
        <Typography.Title level={5}>字典类型</Typography.Title>
        <Space style={{ marginBottom: 12 }} wrap>
          <Input.Search
            placeholder="名称" allowClear style={{ width: 160 }}
            onSearch={(v) => setTypeQuery((q) => ({ ...q, name: v || undefined, page: 1 }))}
          />
          <Select
            placeholder="状态" allowClear style={{ width: 100 }}
            options={STATUS_OPTIONS}
            onChange={(v) => setTypeQuery((q) => ({ ...q, status: v, page: 1 }))}
          />
          <Button icon={<ReloadOutlined />} onClick={() => void loadTypes(typeQuery)}>刷新</Button>
          <Auth code="system:dict:add">
            <Button type="primary" icon={<PlusOutlined />} onClick={openCreateType}>新增</Button>
          </Auth>
        </Space>

        <Table<DictType>
          rowKey="id"
          size="small"
          loading={typeLoading}
          columns={typeColumns}
          dataSource={types}
          scroll={{ x: 520 }}
          rowSelection={{
            type: 'radio',
            selectedRowKeys: selected ? [selected.id] : [],
            onChange: (_keys, rows) => rows[0] && pick(rows[0]),
          }}
          onRow={(row) => ({ onClick: () => pick(row), style: { cursor: 'pointer' } })}
          pagination={{
            current: typeQuery.page, pageSize: typeQuery.size, total: typeTotal, showSizeChanger: true,
            onChange: (page, size) => setTypeQuery((q) => ({ ...q, page, size })),
          }}
        />
      </Col>

      <Col xs={24} xl={14}>
        <Typography.Title level={5}>
          字典项{selected ? `：${selected.name}（${selected.code}）` : ''}
        </Typography.Title>

        {selected ? (
          <>
            <Space style={{ marginBottom: 12 }} wrap>
              <Input.Search
                placeholder="标签" allowClear style={{ width: 160 }}
                onSearch={(v) => setItemQuery((q) => ({ ...q, label: v || undefined, page: 1 }))}
              />
              <Select
                placeholder="状态" allowClear style={{ width: 100 }}
                options={STATUS_OPTIONS}
                onChange={(v) => setItemQuery((q) => ({ ...q, status: v, page: 1 }))}
              />
              <Button icon={<ReloadOutlined />} onClick={() => void loadItems(itemQuery)}>刷新</Button>
              <Auth code="system:dict:add">
                <Button type="primary" icon={<PlusOutlined />} onClick={openCreateItem}>新增</Button>
              </Auth>
              <Button
                icon={<DownloadOutlined />}
                onClick={() => void dictApi.itemExport(itemQuery)}
              >
                导出
              </Button>
            </Space>

            <Table<DictItem>
              rowKey="id"
              size="small"
              loading={itemLoading}
              columns={itemColumns}
              dataSource={items}
              scroll={{ x: 640 }}
              pagination={{
                current: itemQuery.page, pageSize: itemQuery.size, total: itemTotal, showSizeChanger: true,
                onChange: (page, size) => setItemQuery((q) => ({ ...q, page, size })),
              }}
            />
          </>
        ) : (
          <Typography.Text type="secondary">
            请在左侧选择一个字典类型；还没有类型的话，点左栏的「新增」建一个。
          </Typography.Text>
        )}
      </Col>

      <Modal
        title={editingType ? `编辑字典类型：${editingType.name}` : '新增字典类型'}
        open={typeModal}
        onCancel={() => setTypeModal(false)}
        onOk={() => void submitType()}
        destroyOnClose
        width={520}
      >
        <Form form={typeForm} labelCol={{ span: 5 }} wrapperCol={{ span: 18 }}>
          <Form.Item name="name" label="名称" rules={[{ required: true, message: '请输入名称' }]}>
            <Input placeholder="如 用户性别" />
          </Form.Item>
          <Form.Item
            name="code" label="编码"
            rules={[{ required: true, message: '请输入编码' }]}
            extra={editingType ? '编码是字典项的关联键，创建后不可修改' : '如 user_sex'}
          >
            <Input disabled={!!editingType} />
          </Form.Item>
          <Form.Item name="status" label="状态"><Select options={STATUS_OPTIONS} /></Form.Item>
          <Form.Item name="remark" label="备注"><Input.TextArea rows={2} /></Form.Item>
        </Form>
      </Modal>

      <Modal
        title={editingItem ? `编辑字典项：${editingItem.label}` : '新增字典项'}
        open={itemModal}
        onCancel={() => setItemModal(false)}
        onOk={() => void submitItem()}
        destroyOnClose
        width={520}
      >
        <Form form={itemForm} labelCol={{ span: 5 }} wrapperCol={{ span: 18 }}>
          <Form.Item name="type_code" label="类型"><Input disabled /></Form.Item>
          <Form.Item name="label" label="标签" rules={[{ required: true, message: '请输入标签' }]}>
            <Input placeholder="下拉里显示的文字" />
          </Form.Item>
          <Form.Item name="value" label="值" rules={[{ required: true, message: '请输入值' }]}>
            <Input placeholder="存库的值，同类型内不可重复" />
          </Form.Item>
          <Form.Item name="sort" label="排序"><InputNumber min={0} /></Form.Item>
          <Form.Item name="status" label="状态"><Select options={STATUS_OPTIONS} /></Form.Item>
          <Form.Item name="remark" label="备注"><Input.TextArea rows={2} /></Form.Item>
        </Form>
      </Modal>
    </Row>
  );
}
