import { useCallback, useEffect, useState } from 'react';
import { App, Button, DatePicker, Input, Popconfirm, Select, Space, Table, Tag, Typography } from 'antd';
import DownloadOutlined from '@ant-design/icons/es/icons/DownloadOutlined';
import PlusOutlined from '@ant-design/icons/es/icons/PlusOutlined';
import ReloadOutlined from '@ant-design/icons/es/icons/ReloadOutlined';
import type { ColumnsType } from 'antd/es/table';
import { exportApi, type ExportQuery } from '../../../api/export';
import type { ExportKind, ExportTask } from '../../../api/types';
import Auth from '../../../auth/Auth';
import { useAuth } from '../../../auth/AuthContext';
import { useI18n, type I18nKey } from '../../../i18n';
import { AUDIT_MODULES } from '../auditlog/modules';
import { sizeText, visibleKind } from './format';

/** 类型是后端代码注册的，这里只列出来（名字走 i18n，认不出的 kind 直接显示 code）。
 * `perm` = 该 kind 的查看码：**没有这个码就不该看到这个类型**（后端也按同一个码 403） */
const KIND_KEYS: { value: ExportKind; key: I18nKey; perm: string }[] = [
  { value: 'audit-log', key: 'export.kind.audit_log', perm: 'system:auditlog:list' },
  { value: 'login-log', key: 'export.kind.login_log', perm: 'system:loginlog:list' },
];

/** 下标 = 状态值（0 待处理 / 1 执行中 / 2 已完成 / 3 失败 / 4 已取消）：顺序不能动 */
const STATUS: { color: string; key: I18nKey }[] = [
  { color: 'default', key: 'export.status.pending' },
  { color: 'processing', key: 'export.status.running' },
  { color: 'success', key: 'export.status.done' },
  { color: 'error', key: 'export.status.failed' },
  { color: 'warning', key: 'export.status.canceled' },
];

/** 列表里轮询的间隔：任务由定时任务执行（一分钟一次），再快也没用 */
const POLL_MS = 3000;

export default function ExportPage() {
  const { message } = App.useApp();
  const { has } = useAuth();
  const { t } = useI18n();
  const [query, setQuery] = useState<ExportQuery>({ page: 1, size: 10 });
  const [rows, setRows] = useState<ExportTask[]>([]);
  const [total, setTotal] = useState(0);
  const [loading, setLoading] = useState(false);
  // 只看得到自己有查看码的类型（后端也按同一个码 403，这里是不给点）
  const allowed = KIND_KEYS.filter((k) => has(k.perm));
  const [kind, setKind] = useState<ExportKind>('audit-log');
  // 选中项不在「有查看码」的那几个里就回落到第一个可见类型（只有登录记录权限的账号
  // 默认的 audit-log 是看不到的，不能因此把整块发起区关掉）；都不可见才是 undefined
  const pick = visibleKind(allowed, kind);
  const pickKind = (k: ExportKind) => {
    setKind(k);
    setParams({}); // 换类型清筛选：条件不通用（模块只有操作日志有）
  };
  const [params, setParams] = useState<Record<string, unknown>>({});
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
    if (!pick) return;
    setCreating(true);
    try {
      // 空值不发（后端缺省 = 全量）；params 长度后端有 1024 的闸，正常填满也到不了
      const clean = Object.fromEntries(
        Object.entries(params).filter(([, v]) => v !== undefined && v !== ''),
      );
      await exportApi.create(pick.value, clean);
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

  const cancel = async (row: ExportTask) => {
    await exportApi.cancel(row.id);
    message.success(t('export.canceled'));
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
      // 参数为空（`{}` / 空串）= 发起时没设条件，即全量；显示成「全量」比一串空括号好读
      render: (v: string) => (v === '' || v === '{}' ? t('export.all') : v),
    },
    {
      // 进度：执行中才有意义（每批 5000 行回写一次）；取消的那行停在取消时的部分值 ——
      // 总量在分页取数下是不知道的，所以只报行数，不编百分比
      title: t('field.progress'), dataIndex: 'progress_rows', width: 90,
      render: (v: number, row) => (row.status === 1 || row.status === 4 ? v : '-'),
    },
    { title: t('field.row_count'), dataIndex: 'row_count', width: 90, render: (v: number) => (v > 0 ? v : '-') },
    { title: t('field.file_size'), dataIndex: 'file_size', width: 100, render: (v: number) => sizeText(v) },
    { title: t('field.created_at'), dataIndex: 'created_at', width: 170 },
    { title: t('field.finished_at'), dataIndex: 'finished_at', width: 170, render: (v: string | null) => v || '-' },
    { title: t('field.expire_at'), dataIndex: 'expire_at', width: 170 },
    { title: t('field.msg'), dataIndex: 'msg', ellipsis: true },
    {
      // 三个动作（下载 / 取消 / 删除）放得下：150 会把取消挤到换行
      title: t('common.actions'), width: 210, fixed: 'right',
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
          {/* 只给「还没跑完」的行（0/1）：跑完的取消了也没东西可取消，后端也会 409 */}
          {(row.status === 0 || row.status === 1) && (
            <Popconfirm title={t('export.cancel_confirm')} onConfirm={() => void cancel(row)}>
              <Button size="small" type="link" danger>{t('export.cancel')}</Button>
            </Popconfirm>
          )}
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
      {pick ? (
        <Space style={{ marginBottom: 12 }} wrap>
          <Select
            value={pick.value}
            style={{ width: 160 }}
            options={allowed.map((k) => ({ value: k.value, label: t(k.key) }))}
            onChange={pickKind}
          />
          <Input.Search
            placeholder={t('field.username')} allowClear style={{ width: 180 }}
            onSearch={(v) => setParams((p) => ({ ...p, username: v || undefined }))}
          />
          {/* 筛选条件是**按类型给**的：模块只有操作日志有，混着发后端也只会忽略 */}
          {pick.value === 'audit-log' && (
            <Select
              placeholder={t('field.module')} allowClear style={{ width: 140 }}
              options={Object.entries(AUDIT_MODULES).map(([value, key]) => ({ value, label: t(key) }))}
              onChange={(v) => setParams((p) => ({ ...p, module: v }))}
            />
          )}
          <Select
            placeholder={t('field.status')} allowClear style={{ width: 120 }}
            options={[{ value: 1, label: t('common.success') }, { value: 0, label: t('common.failed') }]}
            onChange={(v) => setParams((p) => ({ ...p, status: v }))}
          />
          <DatePicker.RangePicker
            showTime
            placeholder={[t('field.started_at'), t('field.ended_at')]}
            onChange={(v) => setParams((p) => ({
              ...p,
              start: v?.[0]?.format('YYYY-MM-DD HH:mm:ss'),
              end: v?.[1]?.format('YYYY-MM-DD HH:mm:ss'),
            }))}
          />
          <Button type="primary" icon={<PlusOutlined />} loading={creating} onClick={() => void create()}>
            {t('export.new')}
          </Button>
        </Space>
      ) : (
        <Typography.Paragraph type="secondary">{t('export.no_kinds')}</Typography.Paragraph>
      )}
      <Space style={{ marginBottom: 12 }} wrap>
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
        scroll={{ x: 1400 }}
        pagination={{
          current: query.page, pageSize: query.size, total, showSizeChanger: true,
          onChange: (page, size) => setQuery((q) => ({ ...q, page, size })),
        }}
      />
    </>
  );
}
