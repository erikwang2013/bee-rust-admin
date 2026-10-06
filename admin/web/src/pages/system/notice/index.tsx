import { useCallback, useEffect, useState } from 'react';
import { App, Button, Form, Input, Modal, Popconfirm, Select, Space, Table, Tag, Typography } from 'antd';
import { PlusOutlined, ReloadOutlined } from '@ant-design/icons';
import type { ColumnsType } from 'antd/es/table';
import { noticeApi, type NoticeForm, type NoticeQuery } from '../../../api/notice';
import type { Notice } from '../../../api/types';
import Auth from '../../../auth/Auth';

const STATUS_OPTIONS = [{ value: 1, label: '已发布' }, { value: 0, label: '草稿' }];

const statusTag = (v: number) => (
  <Tag color={v === 1 ? 'green' : 'default'}>{v === 1 ? '已发布' : '草稿'}</Tag>
);

export default function NoticePage() {
  const { message } = App.useApp();
  const [form] = Form.useForm<Pick<NoticeForm, 'title' | 'content'>>();
  const [query, setQuery] = useState<NoticeQuery>({ page: 1, size: 10 });
  const [rows, setRows] = useState<Notice[]>([]);
  const [total, setTotal] = useState(0);
  const [loading, setLoading] = useState(false);
  const [editing, setEditing] = useState<Notice | null>(null);
  const [modal, setModal] = useState(false);

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
      message.success('已保存');
    } else {
      await noticeApi.create(body);
      message.success('已保存为草稿，确认无误后再发布');
    }
    setModal(false);
    void load(query);
  };

  const setStatus = async (row: Notice, status: number) => {
    await noticeApi.update(row.id, { title: row.title, content: row.content, status });
    message.success(status === 1 ? '已发布' : '已取消发布');
    void load(query);
  };

  const remove = async (row: Notice) => {
    await noticeApi.remove(row.id);
    message.success('已删除');
    void load(query);
  };

  const columns: ColumnsType<Notice> = [
    { title: 'ID', dataIndex: 'id', width: 70 },
    { title: '标题', dataIndex: 'title' },
    { title: '状态', dataIndex: 'status', width: 90, render: statusTag },
    {
      title: '发布时间', dataIndex: 'published_at', width: 170,
      // 取消发布不清 published_at（留痕：曾发布过），所以草稿行也可能有值
      render: (v: string | null) => v || '-',
    },
    { title: '更新时间', dataIndex: 'updated_at', width: 170 },
    {
      title: '操作', width: 200, fixed: 'right',
      render: (_, row) => (
        <Space>
          <Auth code="system:notice:edit">
            <Button size="small" type="link" onClick={() => openEdit(row)}>编辑</Button>
          </Auth>
          {row.status === 1 ? (
            <Auth code="system:notice:edit">
              <Popconfirm
                title="取消发布后，登录用户将不再在未读里看到它。"
                okText="取消发布"
                onConfirm={() => void setStatus(row, 0)}
              >
                <Button size="small" type="link">取消发布</Button>
              </Popconfirm>
            </Auth>
          ) : (
            <Auth code="system:notice:edit">
              <Button size="small" type="link" onClick={() => void setStatus(row, 1)}>发布</Button>
            </Auth>
          )}
          <Auth code="system:notice:remove">
            <Popconfirm
              title={`确认删除公告「${row.title}」？`}
              description="已读记录会一并删除。"
              okText="删除"
              okButtonProps={{ danger: true }}
              onConfirm={() => void remove(row)}
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
      <Space style={{ marginBottom: 12 }} wrap>
        <Input.Search
          placeholder="标题" allowClear style={{ width: 200 }}
          onSearch={(v) => setQuery((q) => ({ ...q, title: v || undefined, page: 1 }))}
        />
        <Select
          placeholder="状态" allowClear style={{ width: 120 }}
          options={STATUS_OPTIONS}
          onChange={(v) => setQuery((q) => ({ ...q, status: v, page: 1 }))}
        />
        <Button icon={<ReloadOutlined />} onClick={() => void load(query)}>刷新</Button>
        <Auth code="system:notice:add">
          <Button type="primary" icon={<PlusOutlined />} onClick={openCreate}>新增</Button>
        </Auth>
      </Space>
      <Typography.Paragraph type="secondary" style={{ marginBottom: 12 }}>
        新建的公告先存成草稿，草稿对普通用户完全不可见；「发布」之后登录用户才会在顶栏铃铛里收到未读提示。
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
        title={editing ? `编辑公告：${editing.title}` : '新增公告'}
        open={modal}
        onCancel={() => setModal(false)}
        onOk={() => void submit()}
        destroyOnClose
        width={640}
      >
        <Form form={form} labelCol={{ span: 4 }} wrapperCol={{ span: 19 }}>
          <Form.Item
            name="title" label="标题"
            rules={[{ required: true, message: '请输入标题' }, { max: 128, message: '标题最长 128 个字' }]}
          >
            <Input maxLength={128} />
          </Form.Item>
          <Form.Item name="content" label="内容" rules={[{ required: true, message: '请输入内容' }]}>
            <Input.TextArea rows={10} placeholder="纯文本，换行会原样保留" />
          </Form.Item>
        </Form>
      </Modal>
    </>
  );
}
