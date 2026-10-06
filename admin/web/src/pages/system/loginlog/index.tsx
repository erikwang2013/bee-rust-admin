import { useCallback, useEffect, useState } from 'react';
import { App, Button, DatePicker, Input, Popconfirm, Select, Space, Table, Tag } from 'antd';
import { DeleteOutlined, DownloadOutlined, ReloadOutlined } from '@ant-design/icons';
import type { ColumnsType } from 'antd/es/table';
import { loginLogApi, type LoginLogQuery } from '../../../api/loginLog';
import { downloadCsv } from '../../../api/download';
import type { LoginLog } from '../../../api/types';
import Auth from '../../../auth/Auth';
import { useI18n } from '../../../i18n';

export default function LoginLogPage() {
  const { message } = App.useApp();
  const { t } = useI18n();
  const [query, setQuery] = useState<LoginLogQuery>({ page: 1, size: 10 });
  const [rows, setRows] = useState<LoginLog[]>([]);
  const [total, setTotal] = useState(0);
  const [loading, setLoading] = useState(false);

  const load = useCallback(async (q: LoginLogQuery) => {
    setLoading(true);
    try {
      const res = await loginLogApi.list(q);
      setRows(res.list);
      setTotal(res.total);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { void load(query); }, [query, load]);

  const clear = async () => {
    await loginLogApi.clear(query);
    message.success(t('loginlog.cleared'));
    void load(query);
  };

  const columns: ColumnsType<LoginLog> = [
    { title: t('field.id'), dataIndex: 'id', width: 70 },
    { title: t('field.username'), dataIndex: 'username' },
    { title: t('field.ip'), dataIndex: 'ip', width: 140 },
    {
      title: t('field.status'), dataIndex: 'status', width: 80,
      render: (v: number) => (
        <Tag color={v === 1 ? 'green' : 'red'}>{v === 1 ? t('common.success') : t('common.failed')}</Tag>
      ),
    },
    { title: t('field.desc'), dataIndex: 'msg' },
    { title: t('field.user_agent'), dataIndex: 'user_agent', width: 260, ellipsis: true },
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
        <Button icon={<DownloadOutlined />} onClick={() => void downloadCsv('/login-logs/export', query, 'login-logs')}>
          {t('common.export')}
        </Button>
        <Auth code="system:loginlog:remove">
          <Popconfirm title={t('loginlog.clear_confirm')} onConfirm={() => void clear()}>
            <Button danger icon={<DeleteOutlined />}>{t('common.clear')}</Button>
          </Popconfirm>
        </Auth>
      </Space>

      <Table<LoginLog>
        rowKey="id"
        size="small"
        loading={loading}
        columns={columns}
        dataSource={rows}
        scroll={{ x: 1100 }}
        pagination={{
          current: query.page, pageSize: query.size, total, showSizeChanger: true,
          onChange: (page, size) => setQuery((q) => ({ ...q, page, size })),
        }}
      />
    </>
  );
}
