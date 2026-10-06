import { useCallback, useEffect, useState } from 'react';
import {
  App, Button, DatePicker, Form, Input, InputNumber, Modal, Select, Space, Table, Tabs, Tag, Typography,
} from 'antd';
import { ReloadOutlined, ThunderboltOutlined } from '@ant-design/icons';
import type { ColumnsType } from 'antd/es/table';
import { jobApi, type JobLogQuery, type JobQuery } from '../../../api/job';
import type { Job, JobLog } from '../../../api/types';
import Auth from '../../../auth/Auth';
import { intervalText, runResult } from './format';

/** 表单里间隔用 InputNumber（number），提交时再转成协议要求的字符串秒数 */
type JobFormValues = { cron: number; status: number };

const STATUS_OPTIONS = [{ value: 1, label: '启用' }, { value: 0, label: '停用' }];
const RUN_STATUS_OPTIONS = [{ value: 1, label: '成功' }, { value: 0, label: '失败' }];

const runTag = (v: number | null) => (v == null ? '-' : (
  <Tag color={v === 1 ? 'green' : 'red'}>{v === 1 ? '成功' : '失败'}</Tag>
));

export default function JobPage() {
  return (
    <Tabs
      items={[
        { key: 'jobs', label: '任务列表', children: <JobList /> },
        { key: 'logs', label: '执行记录', children: <JobLogs /> },
      ]}
    />
  );
}

function JobList() {
  const { message } = App.useApp();
  const [form] = Form.useForm<JobFormValues>();
  const [query, setQuery] = useState<JobQuery>({ page: 1, size: 10 });
  const [rows, setRows] = useState<Job[]>([]);
  const [total, setTotal] = useState(0);
  const [loading, setLoading] = useState(false);
  const [editing, setEditing] = useState<Job | null>(null);
  const [modal, setModal] = useState(false);
  const [runningId, setRunningId] = useState<number | null>(null);

  const load = useCallback(async (q: JobQuery) => {
    setLoading(true);
    try {
      const res = await jobApi.list(q);
      setRows(res.list);
      setTotal(res.total);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { void load(query); }, [query, load]);

  const openEdit = (row: Job) => {
    setEditing(row);
    form.setFieldsValue({ cron: Number(row.cron), status: row.status });
    setModal(true);
  };

  const submit = async () => {
    if (!editing) return;
    const v = await form.validateFields();
    await jobApi.update(editing.id, { cron: String(v.cron), status: v.status });
    message.success('已保存');
    setModal(false);
    void load(query);
  };

  /** 手动触发：后端同步执行并把结果（含 msg）直接返回，失败时拦截器已提示 409/500 */
  const run = async (row: Job) => {
    setRunningId(row.id);
    try {
      const { ok, text } = runResult(await jobApi.run(row.id));
      if (ok) message.success(text);
      else message.error(text);
      void load(query); // 上次运行时间 / 状态 / 消息都变了
    } finally {
      setRunningId(null);
    }
  };

  const columns: ColumnsType<Job> = [
    { title: 'ID', dataIndex: 'id', width: 70 },
    { title: '名称', dataIndex: 'name', width: 160 },
    { title: 'code', dataIndex: 'code', width: 180 },
    { title: '间隔', dataIndex: 'cron', width: 100, render: (v: string) => intervalText(v) },
    {
      title: '状态', dataIndex: 'status', width: 80,
      render: (v: number) => <Tag color={v === 1 ? 'green' : 'red'}>{v === 1 ? '启用' : '停用'}</Tag>,
    },
    { title: '上次运行', dataIndex: 'last_run_at', width: 170, render: (v: string | null) => v || '从未运行' },
    { title: '上次结果', dataIndex: 'last_status', width: 90, render: runTag },
    { title: '消息', dataIndex: 'last_msg', ellipsis: true },
    {
      title: '操作', width: 160, fixed: 'right',
      render: (_, row) => (
        <Space>
          <Auth code="system:job:edit">
            <Button size="small" type="link" onClick={() => openEdit(row)}>编辑</Button>
          </Auth>
          <Auth code="system:job:edit">
            <Button
              size="small"
              type="link"
              icon={<ThunderboltOutlined />}
              loading={runningId === row.id}
              onClick={() => void run(row)}
            >
              触发一次
            </Button>
          </Auth>
        </Space>
      ),
    },
  ];

  return (
    <>
      <Space style={{ marginBottom: 12 }} wrap>
        <Input.Search
          placeholder="名称" allowClear style={{ width: 180 }}
          onSearch={(v) => setQuery((q) => ({ ...q, name: v || undefined, page: 1 }))}
        />
        <Select
          placeholder="状态" allowClear style={{ width: 110 }}
          options={STATUS_OPTIONS}
          onChange={(v) => setQuery((q) => ({ ...q, status: v, page: 1 }))}
        />
        <Button icon={<ReloadOutlined />} onClick={() => void load(query)}>刷新</Button>
      </Space>
      <Typography.Paragraph type="secondary" style={{ marginBottom: 12 }}>
        任务由代码注册（code 是注册键，库里只放调度与执行记录），这里只能改间隔与启停，不能新增或改名。
      </Typography.Paragraph>

      <Table<Job>
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

      <Modal
        title={editing ? `编辑任务：${editing.name}` : '编辑任务'}
        open={modal}
        onCancel={() => setModal(false)}
        onOk={() => void submit()}
        destroyOnClose
        width={480}
      >
        <Form form={form} labelCol={{ span: 6 }} wrapperCol={{ span: 16 }}>
          <Form.Item label="名称">{editing?.name}</Form.Item>
          <Form.Item label="code">{editing?.code}</Form.Item>
          <Form.Item
            name="cron" label="间隔（秒）"
            rules={[
              { required: true, message: '请输入间隔秒数' },
              {
                validator: (_, v: number) => (
                  Number.isInteger(v) && v > 0 ? Promise.resolve() : Promise.reject(new Error('间隔得是大于 0 的整数秒'))
                ),
              },
            ]}
          >
            <InputNumber min={1} step={60} style={{ width: 180 }} />
          </Form.Item>
          <Form.Item name="status" label="状态"><Select options={STATUS_OPTIONS} /></Form.Item>
        </Form>
      </Modal>
    </>
  );
}

function JobLogs() {
  const [query, setQuery] = useState<JobLogQuery>({ page: 1, size: 10 });
  const [rows, setRows] = useState<JobLog[]>([]);
  const [total, setTotal] = useState(0);
  const [loading, setLoading] = useState(false);

  const load = useCallback(async (q: JobLogQuery) => {
    setLoading(true);
    try {
      const res = await jobApi.logs(q);
      setRows(res.list);
      setTotal(res.total);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { void load(query); }, [query, load]);

  const columns: ColumnsType<JobLog> = [
    { title: 'ID', dataIndex: 'id', width: 70 },
    { title: '任务', dataIndex: 'job_code', width: 180 },
    { title: '开始时间', dataIndex: 'started_at', width: 170 },
    { title: '耗时', dataIndex: 'duration_ms', width: 90, render: (v: number) => `${v} ms` },
    { title: '状态', dataIndex: 'status', width: 80, render: runTag },
    { title: '消息', dataIndex: 'msg', ellipsis: true },
  ];

  return (
    <>
      <Space style={{ marginBottom: 12 }} wrap>
        <Input.Search
          placeholder="任务 code" allowClear style={{ width: 180 }}
          onSearch={(v) => setQuery((q) => ({ ...q, job_code: v || undefined, page: 1 }))}
        />
        <Select
          placeholder="状态" allowClear style={{ width: 110 }}
          options={RUN_STATUS_OPTIONS}
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
      </Space>

      <Table<JobLog>
        rowKey="id"
        size="small"
        loading={loading}
        columns={columns}
        dataSource={rows}
        scroll={{ x: 800 }}
        pagination={{
          current: query.page, pageSize: query.size, total, showSizeChanger: true,
          onChange: (page, size) => setQuery((q) => ({ ...q, page, size })),
        }}
      />
    </>
  );
}
