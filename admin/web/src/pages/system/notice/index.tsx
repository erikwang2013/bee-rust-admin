import { useCallback, useEffect, useState } from 'react';
import { App, Button, Form, Input, Modal, Popconfirm, Select, Space, Table, Tag, Typography } from 'antd';
import { PlusOutlined, ReloadOutlined } from '@ant-design/icons';
import type { ColumnsType } from 'antd/es/table';
import { noticeApi, type NoticeForm, type NoticeQuery } from '../../../api/notice';
import type { Notice } from '../../../api/types';
import Auth from '../../../auth/Auth';
import { useI18n } from '../../../i18n';

export default function NoticePage() {
  const { message } = App.useApp();
  const { t } = useI18n();
  const [form] = Form.useForm<Pick<NoticeForm, 'title' | 'content'>>();
  const [query, setQuery] = useState<NoticeQuery>({ page: 1, size: 10 });
  const [rows, setRows] = useState<Notice[]>([]);
  const [total, setTotal] = useState(0);
  const [loading, setLoading] = useState(false);
  const [editing, setEditing] = useState<Notice | null>(null);
  const [modal, setModal] = useState(false);

  const statusOptions = [
    { value: 1, label: t('notice.published') },
    { value: 0, label: t('notice.draft') },
  ];
  const statusTag = (v: number) => (
    <Tag color={v === 1 ? 'green' : 'default'}>{v === 1 ? t('notice.published') : t('notice.draft')}</Tag>
  );

  const load = useCallback(async (q: NoticeQuery) => {
    setLoading(true);
    try {
      const res = await noticeApi.list(q);
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
    setModal(true);
  };

  const openEdit = (row: Notice) => {
    setEditing(row);
    form.setFieldsValue({ title: row.title, content: row.content });
    setModal(true);
  };

  /** 新建一律先落草稿（发布是列表上的独立动作）；编辑保留原状态，后端对已发布的不会重复盖章 */
  const submit = async () => {
    const v = await form.validateFields();
    const body: NoticeForm = { ...v, status: editing ? editing.status : 0 };
    if (editing) {
      await noticeApi.update(editing.id, body);
      message.success(t('common.saved'));
    } else {
      await noticeApi.create(body);
      message.success(t('notice.saved_as_draft'));
    }
    setModal(false);
    void load(query);
  };

  const setStatus = async (row: Notice, status: number) => {
    await noticeApi.update(row.id, { title: row.title, content: row.content, status });
    message.success(status === 1 ? t('notice.published') : t('notice.unpublish_done'));
    void load(query);
  };

  const remove = async (row: Notice) => {
    await noticeApi.remove(row.id);
    message.success(t('common.deleted'));
    void load(query);
  };

  const columns: ColumnsType<Notice> = [
    { title: t('field.id'), dataIndex: 'id', width: 70 },
    { title: t('field.title'), dataIndex: 'title' },
    { title: t('field.status'), dataIndex: 'status', width: 90, render: statusTag },
    {
      title: t('field.published_at'), dataIndex: 'published_at', width: 170,
      // 取消发布不清 published_at（留痕：曾发布过），所以草稿行也可能有值
      render: (v: string | null) => v || '-',
    },
    { title: t('field.updated_at'), dataIndex: 'updated_at', width: 170 },
    {
      title: t('common.actions'), width: 200, fixed: 'right',
      render: (_, row) => (
        <Space>
          <Auth code="system:notice:edit">
            <Button size="small" type="link" onClick={() => openEdit(row)}>{t('common.edit')}</Button>
          </Auth>
          {row.status === 1 ? (
            <Auth code="system:notice:edit">
              <Popconfirm
                title={t('notice.unpublish_confirm')}
                okText={t('notice.unpublish')}
                onConfirm={() => void setStatus(row, 0)}
              >
                <Button size="small" type="link">{t('notice.unpublish')}</Button>
              </Popconfirm>
            </Auth>
          ) : (
            <Auth code="system:notice:edit">
              <Button size="small" type="link" onClick={() => void setStatus(row, 1)}>{t('notice.publish')}</Button>
            </Auth>
          )}
          <Auth code="system:notice:remove">
            <Popconfirm
              title={t('notice.delete_confirm', { title: row.title })}
              description={t('notice.delete_desc')}
              okText={t('common.delete')}
              okButtonProps={{ danger: true }}
              onConfirm={() => void remove(row)}
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
      <Space style={{ marginBottom: 12 }} wrap>
        <Input.Search
          placeholder={t('field.title')} allowClear style={{ width: 200 }}
          onSearch={(v) => setQuery((q) => ({ ...q, title: v || undefined, page: 1 }))}
        />
        <Select
          placeholder={t('field.status')} allowClear style={{ width: 120 }}
          options={statusOptions}
          onChange={(v) => setQuery((q) => ({ ...q, status: v, page: 1 }))}
        />
        <Button icon={<ReloadOutlined />} onClick={() => void load(query)}>{t('common.refresh')}</Button>
        <Auth code="system:notice:add">
          <Button type="primary" icon={<PlusOutlined />} onClick={openCreate}>{t('common.add')}</Button>
        </Auth>
      </Space>
      <Typography.Paragraph type="secondary" style={{ marginBottom: 12 }}>
        {t('notice.hint')}
      </Typography.Paragraph>

      <Table<Notice>
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
        title={editing ? t('notice.edit_title', { title: editing.title }) : t('notice.create_title')}
        open={modal}
        onCancel={() => setModal(false)}
        onOk={() => void submit()}
        destroyOnClose
        width={640}
      >
        <Form form={form} labelCol={{ span: 4 }} wrapperCol={{ span: 19 }}>
          <Form.Item
            name="title" label={t('field.title')}
            rules={[
              { required: true, whitespace: true, message: t('validate.required', { field: t('field.title') }) },
              { max: 128, message: t('notice.title_max') },
            ]}
          >
            <Input maxLength={128} />
          </Form.Item>
          <Form.Item
            name="content" label={t('field.content')}
            rules={[{ required: true, whitespace: true, message: t('validate.required', { field: t('field.content') }) }]}
          >
            <Input.TextArea rows={10} placeholder={t('notice.content_placeholder')} />
          </Form.Item>
        </Form>
      </Modal>
    </>
  );
}
