import { useCallback, useEffect, useRef, useState, type ChangeEvent } from 'react';
import { App, Button, Input, InputNumber, Modal, Popconfirm, Select, Space, Table, Tag, Typography } from 'antd';
import { DownloadOutlined, LinkOutlined, ReloadOutlined, UploadOutlined } from '@ant-design/icons';
import type { ColumnsType } from 'antd/es/table';
import { attachApi, uploadAttachment, type AttachQuery } from '../../../api/attach';
import { UploadError } from '../../../api/chunkedUpload';
import { noticeApi } from '../../../api/notice';
import type { AttachmentRow, Notice } from '../../../api/types';
import Auth from '../../../auth/Auth';
import { useI18n } from '../../../i18n';
import { sizeText } from '../export/format';

/** 与后端 `ATTACH_EXTS` 同一个白名单（这里只是省得选错文件，真正的闸在服务端） */
const ACCEPT = '.png,.jpg,.jpeg,.gif,.webp,.bmp,.pdf,.txt,.csv,.docx,.xlsx,.pptx,.zip';

export default function AttachPage() {
  const { message } = App.useApp();
  const { t, lang } = useI18n();
  const fileRef = useRef<HTMLInputElement>(null);
  const [query, setQuery] = useState<AttachQuery>({ page: 1, size: 10 });
  const [rows, setRows] = useState<AttachmentRow[]>([]);
  const [total, setTotal] = useState(0);
  const [loading, setLoading] = useState(false);
  const [uploading, setUploading] = useState(false);
  const [percent, setPercent] = useState(0);
  const [binding, setBinding] = useState<AttachmentRow | null>(null);
  const [notices, setNotices] = useState<Notice[]>([]);
  const [target, setTarget] = useState<string | undefined>(undefined);
  const [sort, setSort] = useState(0);

  const load = useCallback(async (q: AttachQuery) => {
    setLoading(true);
    try {
      const res = await attachApi.list(q);
      setRows(res.list);
      setTotal(res.total);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { void load(query); }, [query, load]);

  /** 上传：分片走与头像同一套契约；白名单/大小/配额都在服务端判，这里只报结果 */
  const onPick = async (e: ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    e.target.value = ''; // 允许重复选同一个文件
    if (!file) return;
    setUploading(true);
    setPercent(0);
    try {
      const row = await uploadAttachment(file, {
        locale: lang === 'zh-CN' ? 'zh' : 'en',
        onProgress: (sent, all) => setPercent(Math.round((sent / all) * 100)),
      });
      message.success(t('attach.uploaded', { name: row.name }));
      void load(query);
    } catch (err) {
      // HTTP 层的失败（白名单 415 / 超限 413 / 配额 400）拦截器已提示，这里只管协议异常
      if (err instanceof UploadError) message.error(err.message);
    } finally {
      setUploading(false);
    }
  };

  const openBind = async (row: AttachmentRow) => {
    setBinding(row);
    setTarget(row.entity_id || undefined);
    setSort(row.sort);
    if (notices.length === 0) {
      // 关联对象目前只有公告：一次拉一页够用（选择器就是个下拉）
      const res = await noticeApi.list({ page: 1, size: 100 });
      setNotices(res.list);
    }
  };

  const submitBind = async () => {
    if (!binding) return;
    const id = target ?? '';
    await attachApi.bind(binding.id, { entity_type: id ? 'notice' : '', entity_id: id, sort });
    message.success(id ? t('attach.bind_done') : t('attach.unbind_done'));
    setBinding(null);
    void load(query);
  };

  const remove = async (row: AttachmentRow) => {
    await attachApi.remove(row.id);
    message.success(t('common.deleted'));
    void load(query);
  };

  const columns: ColumnsType<AttachmentRow> = [
    { title: t('field.id'), dataIndex: 'id', width: 70 },
    {
      title: t('field.name'), dataIndex: 'name', ellipsis: true,
      render: (v: string, row) => (
        <Button type="link" size="small" style={{ padding: 0 }} onClick={() => void attachApi.download(row)}>
          {v}
        </Button>
      ),
    },
    { title: t('field.type'), dataIndex: 'ext', width: 80, render: (v: string) => (v ? `.${v}` : '-') },
    { title: t('field.file_size'), dataIndex: 'size', width: 100, render: (v: number) => sizeText(v) },
    {
      title: t('field.sha256'), dataIndex: 'sha256', width: 130,
      render: (v: string) => <span title={v}>{v.slice(0, 12)}…</span>,
    },
    { title: t('field.uploader'), dataIndex: 'admin_name', width: 110, render: (v: string) => v || '-' },
    {
      title: t('field.entity'), dataIndex: 'entity_label', width: 180, ellipsis: true,
      // 绑了已发布公告 = 所有登录用户可下（后端下载鉴权的公开规则），列表上标出来
      render: (v: string, row) =>
        row.entity_type === '' ? t('attach.unbound') : (
          <Space size={4}>
            <span>{v || row.entity_id}</span>
            {row.is_public && <Tag color="green">{t('attach.public')}</Tag>}
          </Space>
        ),
    },
    { title: t('field.sort'), dataIndex: 'sort', width: 70 },
    { title: t('field.created_at'), dataIndex: 'created_at', width: 170 },
    {
      title: t('common.actions'), width: 190, fixed: 'right',
      render: (_, row) => (
        <Space>
          <Button size="small" type="link" icon={<DownloadOutlined />} onClick={() => void attachApi.download(row)}>
            {t('attach.download')}
          </Button>
          <Auth code="system:attach:edit">
            <Button size="small" type="link" icon={<LinkOutlined />} onClick={() => void openBind(row)}>
              {t('attach.bind')}
            </Button>
          </Auth>
          <Auth code="system:attach:remove">
            <Popconfirm title={t('attach.delete_confirm', { name: row.name })} onConfirm={() => void remove(row)}>
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
          placeholder={t('field.name')} allowClear style={{ width: 200 }}
          onSearch={(v) => setQuery((q) => ({ ...q, name: v || undefined, page: 1 }))}
        />
        <Select
          placeholder={t('field.entity')} allowClear style={{ width: 160 }}
          options={[
            { value: 'none', label: t('attach.unbound') },
            { value: 'notice', label: t('attach.entity_notice') },
          ]}
          onChange={(v) => setQuery((q) => ({ ...q, entity_type: v, page: 1 }))}
        />
        <Button icon={<ReloadOutlined />} onClick={() => void load(query)}>{t('common.refresh')}</Button>
        <Auth code="system:attach:add">
          <Button type="primary" icon={<UploadOutlined />} loading={uploading} onClick={() => fileRef.current?.click()}>
            {uploading ? t('attach.uploading', { percent }) : t('attach.upload')}
          </Button>
        </Auth>
        <input ref={fileRef} type="file" accept={ACCEPT} hidden onChange={(e) => void onPick(e)} />
      </Space>
      <Typography.Paragraph type="secondary" style={{ marginBottom: 12 }}>
        {t('attach.hint')}
      </Typography.Paragraph>

      <Table<AttachmentRow>
        rowKey="id"
        size="small"
        loading={loading}
        columns={columns}
        dataSource={rows}
        scroll={{ x: 1280 }}
        pagination={{
          current: query.page, pageSize: query.size, total, showSizeChanger: true,
          onChange: (page, size) => setQuery((q) => ({ ...q, page, size })),
        }}
      />

      <Modal
        title={t('attach.bind_title', { name: binding?.name ?? '' })}
        open={binding !== null}
        onCancel={() => setBinding(null)}
        onOk={() => void submitBind()}
        destroyOnClose
      >
        <Space direction="vertical" style={{ width: '100%' }}>
          <Select
            style={{ width: '100%' }}
            allowClear
            placeholder={t('attach.bind_placeholder')}
            value={target}
            onChange={(v?: string) => setTarget(v)}
            options={notices.map((n) => ({ value: n.id, label: n.title }))}
            optionFilterProp="label"
            showSearch
          />
          <InputNumber
            style={{ width: '100%' }} min={0} max={999}
            value={sort} onChange={(v) => setSort(v ?? 0)}
            addonBefore={t('field.sort')}
          />
          <Typography.Text type="secondary">{t('attach.bind_hint')}</Typography.Text>
        </Space>
      </Modal>
    </>
  );
}
