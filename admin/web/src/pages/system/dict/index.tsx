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
import { useI18n } from '../../../i18n';

export default function DictPage() {
  const { message } = App.useApp();
  const { t } = useI18n();
  const [typeForm] = Form.useForm<DictTypeForm>();
  const [itemForm] = Form.useForm<DictItemForm>();

  const STATUS_OPTIONS = [
    { value: 1, label: t('common.enabled') },
    { value: 0, label: t('common.disabled') },
  ];
  const statusTag = (v: number) => (
    <Tag color={v === 1 ? 'green' : 'red'}>{v === 1 ? t('common.enabled') : t('common.disabled')}</Tag>
  );

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
      message.success(t('common.saved'));
      setSelected((s) => (s && s.id === editingType.id ? { ...s, ...rest } : s));
    } else {
      await dictApi.create(v);
      message.success(t('common.created'));
    }
    setTypeModal(false);
    void loadTypes(typeQuery);
  };

  const removeType = async (row: DictType) => {
    await dictApi.remove(row.id);
    message.success(t('common.deleted'));
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
      message.success(t('common.saved'));
    } else {
      await dictApi.itemCreate(v);
      message.success(t('common.created'));
    }
    dictCache.delete(v.type_code); // 别处下拉若已缓存该 code，丢弃旧选项
    setItemModal(false);
    void loadItems(itemQuery);
  };

  const typeColumns: ColumnsType<DictType> = [
    { title: t('field.name'), dataIndex: 'name' },
    { title: t('field.code'), dataIndex: 'code' },
    { title: t('field.status'), dataIndex: 'status', width: 80, render: statusTag },
    { title: t('field.remark'), dataIndex: 'remark', ellipsis: true },
    {
      title: t('common.actions'), width: 110, fixed: 'right',
      render: (_, row) => (
        <Space>
          <Auth code="system:dict:edit">
            <Button size="small" type="link" onClick={() => openEditType(row)}>{t('common.edit')}</Button>
          </Auth>
          <Auth code="system:dict:remove">
            <Popconfirm
              title={t('dict.type_delete_confirm', { name: row.name })}
              description={t('dict.type_delete_desc')}
              okText={t('common.delete')}
              okButtonProps={{ danger: true }}
              onConfirm={() => void removeType(row)}
            >
              <Button size="small" type="link" danger>{t('common.delete')}</Button>
            </Popconfirm>
          </Auth>
        </Space>
      ),
    },
  ];

  const itemColumns: ColumnsType<DictItem> = [
    { title: t('field.label'), dataIndex: 'label' },
    { title: t('field.value'), dataIndex: 'value' },
    { title: t('field.sort'), dataIndex: 'sort', width: 70 },
    { title: t('field.status'), dataIndex: 'status', width: 80, render: statusTag },
    { title: t('field.remark'), dataIndex: 'remark', ellipsis: true },
    {
      title: t('common.actions'), width: 110, fixed: 'right',
      render: (_, row) => (
        <Space>
          <Auth code="system:dict:edit">
            <Button size="small" type="link" onClick={() => openEditItem(row)}>{t('common.edit')}</Button>
          </Auth>
          <Auth code="system:dict:remove">
            <Popconfirm
              title={t('dict.item_delete_confirm')}
              okText={t('common.delete')}
              okButtonProps={{ danger: true }}
              onConfirm={async () => {
                await dictApi.itemRemove(row.id);
                message.success(t('common.deleted'));
                dictCache.delete(row.type_code);
                void loadItems(itemQuery);
              }}
            >
              <Button size="small" type="link" danger>{t('common.delete')}</Button>
            </Popconfirm>
          </Auth>
        </Space>
      ),
    },
  ];

  return (
    <Row gutter={16}>
      <Col xs={24} xl={10}>
        <Typography.Title level={5}>{t('dict.types_title')}</Typography.Title>
        <Space style={{ marginBottom: 12 }} wrap>
          <Input.Search
            placeholder={t('field.name')} allowClear style={{ width: 160 }}
            onSearch={(v) => setTypeQuery((q) => ({ ...q, name: v || undefined, page: 1 }))}
          />
          <Select
            placeholder={t('field.status')} allowClear style={{ width: 100 }}
            options={STATUS_OPTIONS}
            onChange={(v) => setTypeQuery((q) => ({ ...q, status: v, page: 1 }))}
          />
          <Button icon={<ReloadOutlined />} onClick={() => void loadTypes(typeQuery)}>{t('common.refresh')}</Button>
          <Auth code="system:dict:add">
            <Button type="primary" icon={<PlusOutlined />} onClick={openCreateType}>{t('common.add')}</Button>
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
          {selected
            ? t('dict.items_title_selected', { name: selected.name, code: selected.code })
            : t('dict.items_title')}
        </Typography.Title>

        {selected ? (
          <>
            <Space style={{ marginBottom: 12 }} wrap>
              <Input.Search
                placeholder={t('field.label')} allowClear style={{ width: 160 }}
                onSearch={(v) => setItemQuery((q) => ({ ...q, label: v || undefined, page: 1 }))}
              />
              <Select
                placeholder={t('field.status')} allowClear style={{ width: 100 }}
                options={STATUS_OPTIONS}
                onChange={(v) => setItemQuery((q) => ({ ...q, status: v, page: 1 }))}
              />
              <Button icon={<ReloadOutlined />} onClick={() => void loadItems(itemQuery)}>{t('common.refresh')}</Button>
              <Auth code="system:dict:add">
                <Button type="primary" icon={<PlusOutlined />} onClick={openCreateItem}>{t('common.add')}</Button>
              </Auth>
              <Button
                icon={<DownloadOutlined />}
                onClick={() => void dictApi.itemExport(itemQuery)}
              >
                {t('common.export')}
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
            {t('dict.empty_hint')}
          </Typography.Text>
        )}
      </Col>

      <Modal
        title={editingType
          ? t('dict.type_edit_title', { name: editingType.name })
          : t('dict.type_create_title')}
        open={typeModal}
        onCancel={() => setTypeModal(false)}
        onOk={() => void submitType()}
        destroyOnClose
        width={520}
      >
        <Form form={typeForm} labelCol={{ span: 5 }} wrapperCol={{ span: 18 }}>
          <Form.Item
            name="name" label={t('field.name')}
            rules={[{ required: true, message: t('validate.required', { field: t('field.name') }) }]}
          >
            <Input placeholder={t('dict.name_example')} />
          </Form.Item>
          <Form.Item
            name="code" label={t('field.code')}
            rules={[{ required: true, message: t('validate.required', { field: t('field.code') }) }]}
            extra={editingType ? t('dict.code_hint_locked') : t('dict.code_example')}
          >
            <Input disabled={!!editingType} />
          </Form.Item>
          <Form.Item name="status" label={t('field.status')}><Select options={STATUS_OPTIONS} /></Form.Item>
          <Form.Item name="remark" label={t('field.remark')}><Input.TextArea rows={2} /></Form.Item>
        </Form>
      </Modal>

      <Modal
        title={editingItem
          ? t('dict.item_edit_title', { label: editingItem.label })
          : t('dict.item_create_title')}
        open={itemModal}
        onCancel={() => setItemModal(false)}
        onOk={() => void submitItem()}
        destroyOnClose
        width={520}
      >
        <Form form={itemForm} labelCol={{ span: 5 }} wrapperCol={{ span: 18 }}>
          <Form.Item name="type_code" label={t('field.dict_type')}><Input disabled /></Form.Item>
          <Form.Item
            name="label" label={t('field.label')}
            rules={[{ required: true, message: t('validate.required', { field: t('field.label') }) }]}
          >
            <Input placeholder={t('dict.label_placeholder')} />
          </Form.Item>
          <Form.Item
            name="value" label={t('field.value')}
            rules={[{ required: true, message: t('validate.required', { field: t('field.value') }) }]}
          >
            <Input placeholder={t('dict.value_placeholder')} />
          </Form.Item>
          <Form.Item name="sort" label={t('field.sort')}><InputNumber min={0} /></Form.Item>
          <Form.Item name="status" label={t('field.status')}><Select options={STATUS_OPTIONS} /></Form.Item>
          <Form.Item name="remark" label={t('field.remark')}><Input.TextArea rows={2} /></Form.Item>
        </Form>
      </Modal>
    </Row>
  );
}
