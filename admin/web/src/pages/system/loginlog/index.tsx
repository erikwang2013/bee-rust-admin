import { useCallback, useEffect, useState } from 'react';
import { App, Button, DatePicker, Input, Popconfirm, Select, Space, Table, Tag } from 'antd';
import { DeleteOutlined, ReloadOutlined } from '@ant-design/icons';
import type { ColumnsType } from 'antd/es/table';
import { loginLogApi, type LoginLogQuery } from '../../../api/loginLog';
import type { LoginLog } from '../../../api/types';
import Auth from '../../../auth/Auth';

export default function LoginLogPage() {
  const { message } = App.useApp();
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
    message.success('已清空');
    void load(query);
  };

  const columns: ColumnsType<LoginLog> = [
    { title: 'ID', dataIndex: 'id', width: 70 },
    { title: '用户名', dataIndex: 'username' },
    { title: 'IP', dataIndex: 'ip', width: 140 },
    {
      title: '状态', dataIndex: 'status', width: 80,
      render: (v: number) => <Tag color={v === 1 ? 'green' : 'red'}>{v === 1 ? '成功' : '失败'}</Tag>,
    },
    { title: '说明', dataIndex: 'msg' },
    { title: 'User-Agent', dataIndex: 'user_agent', width: 260, ellipsis: true },
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
        <Auth code="system:loginlog:remove">
          <Popconfirm title="确认清空当前筛选条件下的登录记录？" onConfirm={() => void clear()}>
            <Button danger icon={<DeleteOutlined />}>清空</Button>
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
