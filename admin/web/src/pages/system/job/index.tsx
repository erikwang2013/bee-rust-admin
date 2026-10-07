import { useCallback, useEffect, useState } from 'react';
import {
  App, Button, DatePicker, Form, Input, InputNumber, Modal, Select, Space, Table, Tabs, Tag, Typography,
} from 'antd';
import { ReloadOutlined, ThunderboltOutlined } from '@ant-design/icons';
import type { ColumnsType } from 'antd/es/table';
import { jobApi, type JobLogQuery, type JobQuery } from '../../../api/job';
import type { Job, JobLog } from '../../../api/types';
import Auth from '../../../auth/Auth';
import { useI18n, type TFunc } from '../../../i18n';
import { intervalText, runResult } from './format';

/** 表单里间隔用 InputNumber（number），提交时再转成协议要求的字符串秒数 */
type JobFormValues = { cron: number; status: number };

const runTag = (t: TFunc, v: number | null) => (v == null ? '-' : (
  <Tag color={v === 1 ? 'green' : 'red'}>{v === 1 ? t('common.success') : t('common.failed')}</Tag>
));

export default function JobPage() {
  const { t } = useI18n();
  return (
    <Tabs
      items={[
        { key: 'jobs', label: t('job.tab_list'), children: <JobList /> },
        { key: 'logs', label: t('job.tab_logs'), children: <JobLogs /> },
      ]}
    />
  );
}

function JobList() {
  const { message } = App.useApp();
  const { t } = useI18n();
  const statusOptions = [
    { value: 1, label: t('common.enabled') },
    { value: 0, label: t('job.stopped') },
  ];
  const [form] = Form.useForm<JobFormValues>();
  const [query, setQuery] = useState<JobQuery>({ page: 1, size: 10 });
  const [rows, setRows] = useState<Job[]>([]);
  const [total, setTotal] = useState(0);
  const [loading, setLoading] = useState(false);
  const [editing, setEditing] = useState<Job | null>(null);
  const [modal, setModal] = useState(false);
  const [runningId, setRunningId] = useState<string | null>(null);

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
    message.success(t('common.saved'));
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
    { title: t('field.id'), dataIndex: 'id', width: 70 },
    { title: t('field.job_name'), dataIndex: 'name', width: 160 },
    { title: t('field.job_code'), dataIndex: 'code', width: 180 },
    { title: t('field.job_interval'), dataIndex: 'cron', width: 100, render: (v: string) => intervalText(v) },
    {
      title: t('field.status'), dataIndex: 'status', width: 80,
      render: (v: number) => (
        <Tag color={v === 1 ? 'green' : 'red'}>{v === 1 ? t('common.enabled') : t('job.stopped')}</Tag>
      ),
    },
    { title: t('field.last_run'), dataIndex: 'last_run_at', width: 170, render: (v: string | null) => v || t('job.never_run') },
    { title: t('field.last_result'), dataIndex: 'last_status', width: 90, render: (v: number | null) => runTag(t, v) },
    { title: t('field.msg'), dataIndex: 'last_msg', ellipsis: true },
    {
      title: t('common.actions'), width: 160, fixed: 'right',
      render: (_, row) => (
        <Space>
          <Auth code="system:job:edit">
            <Button size="small" type="link" onClick={() => openEdit(row)}>{t('common.edit')}</Button>
          </Auth>
          <Auth code="system:job:edit">
            <Button
              size="small"
              type="link"
              icon={<ThunderboltOutlined />}
              loading={runningId === row.id}
              onClick={() => void run(row)}
            >
              {t('job.trigger')}
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
          placeholder={t('field.job_name')} allowClear style={{ width: 180 }}
          onSearch={(v) => setQuery((q) => ({ ...q, name: v || undefined, page: 1 }))}
        />
        <Select
          placeholder={t('field.status')} allowClear style={{ width: 110 }}
          options={statusOptions}
          onChange={(v) => setQuery((q) => ({ ...q, status: v, page: 1 }))}
        />
        <Button icon={<ReloadOutlined />} onClick={() => void load(query)}>{t('common.refresh')}</Button>
      </Space>
      <Typography.Paragraph type="secondary" style={{ marginBottom: 12 }}>
        {t('job.hint')}
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
        title={editing ? t('job.edit_title', { name: editing.name }) : t('job.edit_title_plain')}
        open={modal}
        onCancel={() => setModal(false)}
        onOk={() => void submit()}
        destroyOnClose
        width={480}
      >
        <Form form={form} labelCol={{ span: 6 }} wrapperCol={{ span: 16 }}>
          <Form.Item label={t('field.job_name')}>{editing?.name}</Form.Item>
          <Form.Item label={t('field.job_code')}>{editing?.code}</Form.Item>
          <Form.Item
            name="cron" label={t('field.job_interval')}
            rules={[
              { required: true, message: t('job.interval_required') },
              {
                validator: (_, v: number) => (
                  Number.isInteger(v) && v > 0
                    ? Promise.resolve()
                    : Promise.reject(new Error(t('job.interval_invalid')))
                ),
              },
            ]}
          >
            <InputNumber min={1} step={60} style={{ width: 180 }} />
          </Form.Item>
          <Form.Item name="status" label={t('field.status')}><Select options={statusOptions} /></Form.Item>
        </Form>
      </Modal>
    </>
  );
}

function JobLogs() {
  const { t } = useI18n();
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
    { title: t('field.id'), dataIndex: 'id', width: 70 },
    { title: t('field.job_code'), dataIndex: 'job_code', width: 180 },
    { title: t('field.started_at'), dataIndex: 'started_at', width: 170 },
    { title: t('field.duration'), dataIndex: 'duration_ms', width: 90, render: (v: number) => `${v} ms` },
    { title: t('field.status'), dataIndex: 'status', width: 80, render: (v: number | null) => runTag(t, v) },
    { title: t('field.msg'), dataIndex: 'msg', ellipsis: true },
  ];

  return (
    <>
      <Space style={{ marginBottom: 12 }} wrap>
        <Input.Search
          placeholder={t('job.code_placeholder')} allowClear style={{ width: 180 }}
          onSearch={(v) => setQuery((q) => ({ ...q, job_code: v || undefined, page: 1 }))}
        />
        <Select
          placeholder={t('field.status')} allowClear style={{ width: 110 }}
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
