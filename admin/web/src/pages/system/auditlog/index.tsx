import { useCallback, useEffect, useState } from 'react';
import { App, Button, DatePicker, Input, Popconfirm, Select, Space, Table, Tag } from 'antd';
import { DeleteOutlined, DownloadOutlined, ReloadOutlined } from '@ant-design/icons';
import type { ColumnsType } from 'antd/es/table';
import { auditLogApi, type AuditLogQuery } from '../../../api/auditLog';
import { downloadCsv } from '../../../api/download';
import type { AuditLog } from '../../../api/types';
import Auth from '../../../auth/Auth';

/** 与后端 audit.rs 的 module_action 取值一一对应（未列出的路径归 other）。 */
const MODULES = [
  { value: 'admin', label: '管理员' },
  { value: 'role', label: '角色' },
  { value: 'menu', label: '菜单' },
  { value: 'dept', label: '部门' },
  { value: 'loginlog', label: '登录记录' },
  { value: 'auditlog', label: '操作日志' },
  { value: 'auth', label: '认证' },
  { value: 'other', label: '其他' },
];

export default function AuditLogPage() {
  const { message } = App.useApp();
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
    message.success(`已清空 ${res.deleted} 条`);
    void load(query);
  };

  const columns: ColumnsType<AuditLog> = [
    { title: 'ID', dataIndex: 'id', width: 70 },
    { title: '用户名', dataIndex: 'username' },
    { title: '模块', dataIndex: 'module', width: 100 },
    { title: '动作', dataIndex: 'action', width: 140 },
    {
      title: '请求', dataIndex: 'path', width: 260, ellipsis: true,
      render: (v: string, row) => <span>{row.method} {v}</span>,
    },
    {
      title: '状态', dataIndex: 'status', width: 80,
      render: (v: number) => <Tag color={v === 1 ? 'green' : 'red'}>{v === 1 ? '成功' : '失败'}</Tag>,
    },
    { title: '说明', dataIndex: 'msg' },
    {
      title: '耗时', dataIndex: 'duration_ms', width: 90,
      render: (v: number) => `${v} ms`,
    },
    { title: 'IP', dataIndex: 'ip', width: 140 },
    { title: '时间', dataIndex: 'created_at', width: 170 },
  ];

  return (
    <>
      <Space style={{ marginBottom: 16 }} wrap>
        <Input.Search
          placeholder="用户名" allowClear style={{ width: 200 }}
          onSearch={(v) => setQuery((q) => ({ ...q, username: v || undefined, page: 1 }))}
        />
        <Select
          placeholder="模块" allowClear style={{ width: 140 }}
          options={MODULES}
          onChange={(v) => setQuery((q) => ({ ...q, module: v, page: 1 }))}
        />
        <Select
          placeholder="状态" allowClear style={{ width: 120 }}
          options={[{ value: 1, label: '成功' }, { value: 0, label: '失败' }]}
          onChange={(v) => setQuery((q) => ({ ...q, status: v, page: 1 }))}
        />
        <DatePicker.RangePicker
          showTime
          placeholder={['开始时间', '结束时间']}
          onChange={(v) => setQuery((q) => ({
            ...q,
            start: v?.[0]?.format('YYYY-MM-DD HH:mm:ss'),
            end: v?.[1]?.format('YYYY-MM-DD HH:mm:ss'),
            page: 1,
          }))}
        />
        <Button icon={<ReloadOutlined />} onClick={() => void load(query)}>刷新</Button>
        <Button icon={<DownloadOutlined />} onClick={() => void downloadCsv('/audit-logs/export', query, 'audit-logs')}>
          导出
        </Button>
        <Auth code="system:auditlog:remove">
          <Popconfirm title="确认清空当前筛选条件下的操作日志？" onConfirm={() => void clear()}>
            <Button danger icon={<DeleteOutlined />}>清空</Button>
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
