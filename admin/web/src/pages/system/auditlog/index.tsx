import { useCallback, useEffect, useState } from 'react';
import { App, Button, DatePicker, Input, Popconfirm, Select, Space, Table, Tag } from 'antd';
import { DeleteOutlined, DownloadOutlined, ReloadOutlined } from '@ant-design/icons';
import type { ColumnsType } from 'antd/es/table';
import { auditLogApi, type AuditLogQuery } from '../../../api/auditLog';
import { downloadCsv } from '../../../api/download';
import type { AuditLog } from '../../../api/types';
import Auth from '../../../auth/Auth';
import { useI18n, type I18nKey } from '../../../i18n';

/** 与后端 audit.rs 的 module_action 取值一一对应（未列出的路径归 other）。 */
const MODULE_KEYS: Record<string, I18nKey> = {
  admin: 'auditlog.module.admin',
  role: 'auditlog.module.role',
  menu: 'auditlog.module.menu',
  dept: 'auditlog.module.dept',
  loginlog: 'auditlog.module.loginlog',
  auditlog: 'auditlog.module.auditlog',
  auth: 'auditlog.module.auth',
  other: 'auditlog.module.other',
};

export default function AuditLogPage() {
  const { message } = App.useApp();
  const { t } = useI18n();
  const [query, setQuery] = useState<AuditLogQuery>({ page: 1, size: 10 });
  const [rows, setRows] = useState<AuditLog[]>([]);
  const [total, setTotal] = useState(0);
  const [loading, setLoading] = useState(false);

  const load = useCallback(async (q: AuditLogQuery) => {
    setLoading(true);
    try {
      const res = await auditLogApi.list(q);
      setRows(res.list);
      setTotal(res.total);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { void load(query); }, [query, load]);

  const clear = async () => {
    const res = await auditLogApi.clear(query);
    message.success(t('auditlog.cleared', { n: res.deleted }));
    void load(query);
  };

  const columns: ColumnsType<AuditLog> = [
    { title: t('field.id'), dataIndex: 'id', width: 70 },
    { title: t('field.username'), dataIndex: 'username' },
    { title: t('field.module'), dataIndex: 'module', width: 100 },
    { title: t('field.action'), dataIndex: 'action', width: 140 },
    {
      title: t('field.request'), dataIndex: 'path', width: 260, ellipsis: true,
      render: (v: string, row) => <span>{row.method} {v}</span>,
    },
    {
      title: t('field.status'), dataIndex: 'status', width: 80,
      render: (v: number) => (
        <Tag color={v === 1 ? 'green' : 'red'}>{v === 1 ? t('common.success') : t('common.failed')}</Tag>
      ),
    },
    { title: t('field.desc'), dataIndex: 'msg' },
    {
      title: t('field.duration'), dataIndex: 'duration_ms', width: 90,
      render: (v: number) => `${v} ms`,
    },
    { title: t('field.ip'), dataIndex: 'ip', width: 140 },
    { title: t('field.time'), dataIndex: 'created_at', width: 170 },
  ];

  return (
    <>
      <Space style={{ marginBottom: 16 }} wrap>
        <Input.Search
          placeholder={t('field.username')} allowClear style={{ width: 200 }}
          onSearch={(v) => setQuery((q) => ({ ...q, username: v || undefined, page: 1 }))}
        />
        <Select
          placeholder={t('field.module')} allowClear style={{ width: 140 }}
          options={Object.entries(MODULE_KEYS).map(([value, key]) => ({ value, label: t(key) }))}
          onChange={(v) => setQuery((q) => ({ ...q, module: v, page: 1 }))}
        />
        <Select
          placeholder={t('field.status')} allowClear style={{ width: 120 }}
          options={[{ value: 1, label: t('common.success') }, { value: 0, label: t('common.failed') }]}
          onChange={(v) => setQuery((q) => ({ ...q, status: v, page: 1 }))}
        />
        <DatePicker.RangePicker
          showTime
          placeholder={[t('field.started_at'), t('field.ended_at')]}
          onChange={(v) => setQuery((q) => ({
            ...q,
            start: v?.[0]?.format('YYYY-MM-DD HH:mm:ss'),
            end: v?.[1]?.format('YYYY-MM-DD HH:mm:ss'),
            page: 1,
          }))}
        />
        <Button icon={<ReloadOutlined />} onClick={() => void load(query)}>{t('common.refresh')}</Button>
        <Button icon={<DownloadOutlined />} onClick={() => void downloadCsv('/audit-logs/export', query, 'audit-logs')}>
          {t('common.export')}
        </Button>
        <Auth code="system:auditlog:remove">
          <Popconfirm title={t('auditlog.clear_confirm')} onConfirm={() => void clear()}>
            <Button danger icon={<DeleteOutlined />}>{t('common.clear')}</Button>
          </Popconfirm>
        </Auth>
      </Space>

      <Table<AuditLog>
        rowKey="id"
        size="small"
        loading={loading}
        columns={columns}
        dataSource={rows}
        scroll={{ x: 1400 }}
        pagination={{
          current: query.page, pageSize: query.size, total, showSizeChanger: true,
          onChange: (page, size) => setQuery((q) => ({ ...q, page, size })),
        }}
      />
    </>
  );
}
