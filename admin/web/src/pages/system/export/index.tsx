import { useCallback, useEffect, useState } from 'react';
import { App, Button, Popconfirm, Select, Space, Table, Tag, Typography } from 'antd';
import { DownloadOutlined, PlusOutlined, ReloadOutlined } from '@ant-design/icons';
import type { ColumnsType } from 'antd/es/table';
import { exportApi, type ExportQuery } from '../../../api/export';
import type { ExportKind, ExportTask } from '../../../api/types';
import Auth from '../../../auth/Auth';
import { useI18n, type I18nKey } from '../../../i18n';
import { sizeText } from './format';

/** 类型是后端代码注册的，这里只列出来（名字走 i18n，认不出的 kind 直接显示 code） */
const KIND_KEYS: { value: ExportKind; key: I18nKey }[] = [
  { value: 'audit-log', key: 'export.kind.audit_log' },
  { value: 'login-log', key: 'export.kind.login_log' },
];

const STATUS: { color: string; key: I18nKey }[] = [
  { color: 'default', key: 'export.status.pending' },
  { color: 'processing', key: 'export.status.running' },
  { color: 'success', key: 'export.status.done' },
  { color: 'error', key: 'export.status.failed' },
];

/** 列表里轮询的间隔：任务由定时任务执行（一分钟一次），再快也没用 */
const POLL_MS = 3000;

export default function ExportPage() {
  const { message } = App.useApp();
  const { t } = useI18n();
  const [query, setQuery] = useState<ExportQuery>({ page: 1, size: 10 });
  const [rows, setRows] = useState<ExportTask[]>([]);
  const [total, setTotal] = useState(0);
  const [loading, setLoading] = useState(false);
  const [kind, setKind] = useState<ExportKind>('audit-log');
  const [creating, setCreating] = useState(false);

  /** quiet = 轮询刷新：不闪加载态（每 3 秒盖一层遮罩很难受） */
  const load = useCallback(async (q: ExportQuery, quiet = false) => {
    if (!quiet) setLoading(true);
    try {
      const res = await exportApi.list(q);
      setRows(res.list);
      setTotal(res.total);
    } finally {
      if (!quiet) setLoading(false);
    }
  }, []);

  useEffect(() => { void load(query); }, [query, load]);

  // 还有待处理/执行中的就自动刷新：发起后不用手点，跑完（或失败）自然停
  const busy = rows.some((r) => r.status === 0 || r.status === 1);
  useEffect(() => {
    if (!busy) return;
    const timer = setInterval(() => void load(query, true), POLL_MS);
    return () => clearInterval(timer);
  }, [busy, query, load]);

  const create = async () => {
    setCreating(true);
    try {
      await exportApi.create(kind);
      message.success(t('export.created'));
      void load(query);
    } finally {
      setCreating(false);
    }
  };

  const remove = async (row: ExportTask) => {
    await exportApi.remove(row.id);
    message.success(t('common.deleted'));
    void load(query);
  };

  const columns: ColumnsType<ExportTask> = [
    { title: t('field.id'), dataIndex: 'id', width: 80 },
    {
      title: t('field.kind'), dataIndex: 'kind', width: 110,
      render: (v: string) => {
        const hit = KIND_KEYS.find((k) => k.value === v);
        return hit ? t(hit.key) : v;
      },
    },
    {
      title: t('field.status'), dataIndex: 'status', width: 90,
      render: (v: number) => {
        const s = STATUS[v] ?? STATUS[0];
        return <Tag color={s.color}>{t(s.key)}</Tag>;
      },
    },
    {
      title: t('field.params'), dataIndex: 'params', width: 140, ellipsis: true,
      // 导出中心只发全量，`{}` 显示成「全量」比一串空括号好读
      render: (v: string) => (v === '' || v === '{}' ? t('export.all') : v),
    },
    { title: t('field.row_count'), dataIndex: 'row_count', width: 90, render: (v: number) => (v > 0 ? v : '-') },
    { title: t('field.file_size'), dataIndex: 'file_size', width: 100, render: (v: number) => sizeText(v) },
    { title: t('field.created_at'), dataIndex: 'created_at', width: 170 },
    { title: t('field.finished_at'), dataIndex: 'finished_at', width: 170, render: (v: string | null) => v || '-' },
    { title: t('field.expire_at'), dataIndex: 'expire_at', width: 170 },
    { title: t('field.msg'), dataIndex: 'msg', ellipsis: true },
    {
      title: t('common.actions'), width: 150, fixed: 'right',
      render: (_, row) => (
        <Space>
          <Button
            size="small"
            type="link"
            icon={<DownloadOutlined />}
            // 只有成品能下：没跑完的下了也是一份空文件
            disabled={row.status !== 2}
            onClick={() => void exportApi.download(row)}
          >
            {t('export.download')}
          </Button>
          <Auth code="system:export:remove">
            <Popconfirm title={t('export.delete_confirm')} onConfirm={() => void remove(row)}>
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
        <Select
          value={kind}
          style={{ width: 160 }}
          options={KIND_KEYS.map((k) => ({ value: k.value, label: t(k.key) }))}
          onChange={setKind}
        />
        <Button type="primary" icon={<PlusOutlined />} loading={creating} onClick={() => void create()}>
          {t('export.new')}
        </Button>
        <Select
          placeholder={t('field.status')} allowClear style={{ width: 120 }}
          options={STATUS.map((s, v) => ({ value: v, label: t(s.key) }))}
          onChange={(v) => setQuery((q) => ({ ...q, status: v, page: 1 }))}
        />
        <Button icon={<ReloadOutlined />} onClick={() => void load(query)}>{t('common.refresh')}</Button>
      </Space>
      <Typography.Paragraph type="secondary" style={{ marginBottom: 12 }}>
        {t('export.hint')}
      </Typography.Paragraph>

      <Table<ExportTask>
        rowKey="id"
        size="small"
        loading={loading}
        columns={columns}
        dataSource={rows}
        scroll={{ x: 1300 }}
        pagination={{
          current: query.page, pageSize: query.size, total, showSizeChanger: true,
          onChange: (page, size) => setQuery((q) => ({ ...q, page, size })),
        }}
      />
    </>
  );
}
