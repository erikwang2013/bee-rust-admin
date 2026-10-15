import { useState } from 'react';
import { App, Button, Modal, Space, Table, Tag, Typography, Upload } from 'antd';
import { DownloadOutlined, FileTextOutlined, UploadOutlined } from '@ant-design/icons';
import type { ColumnsType } from 'antd/es/table';
import { downloadCsv } from '../../../api/download';
import { importApi } from '../../../api/import';
import type { ImportKind, ImportReport, ImportRow } from '../../../api/types';
import Auth from '../../../auth/Auth';
import { useI18n } from '../../../i18n';
import { summarize } from './report';

/**
 * CSV 导入按钮 + 弹窗（项 3）：管理员 / 部门 / 字典项三个页面共用。
 * 模板下的是后端那份（表头即契约）；上传后把后端逐行报告原样铺开（行号 + 原因）。
 * 权限复用各模块的 `:add` 码（传 `perm`），不新增权限码。
 */
export default function ImportButton({
  kind, perm, onDone,
}: {
  kind: ImportKind;
  /** 控制按钮显隐的权限码（后端也按这个码拦） */
  perm: string;
  /** 导入成功后刷新列表 */
  onDone?: () => void;
}) {
  const { t } = useI18n();
  const { message } = App.useApp();
  const [open, setOpen] = useState(false);
  const [file, setFile] = useState<File | null>(null);
  const [report, setReport] = useState<ImportReport | null>(null);
  const [loading, setLoading] = useState(false);

  const close = () => {
    setOpen(false);
    setFile(null);
    setReport(null);
  };

  const submit = async () => {
    if (!file) return;
    setLoading(true);
    try {
      const r = await importApi.upload(kind, file);
      setReport(r);
      const s = summarize(r);
      if (s.level === 'success') {
        message.success(t('import.done', { n: r.created }));
        onDone?.();
      } else if (s.level === 'warn') {
        message.warning(t('import.empty'));
      } else {
        message.error(t('import.bad', { n: s.bad }));
      }
    } finally {
      setLoading(false);
    }
  };

  const columns: ColumnsType<ImportRow> = [
    { title: t('import.line'), dataIndex: 'line', width: 80 },
    {
      title: t('import.result'),
      dataIndex: 'ok',
      width: 100,
      render: (ok: boolean) => (
        <Tag color={ok ? 'green' : 'red'}>{ok ? t('import.ok') : t('import.reject')}</Tag>
      ),
    },
    { title: t('import.reason'), dataIndex: 'msg' },
  ];

  return (
    <>
      <Auth code={perm}>
        <Button icon={<UploadOutlined />} onClick={() => setOpen(true)}>{t('common.import')}</Button>
      </Auth>
      <Modal title={t('import.title')} open={open} onCancel={close} footer={null} width={720} destroyOnClose>
        <Space direction="vertical" size="middle" style={{ width: '100%' }}>
          <Typography.Text type="secondary">{t('import.hint')}</Typography.Text>
          <Space wrap>
            <Button
              icon={<DownloadOutlined />}
              onClick={() => void downloadCsv('/imports/template', { kind }, `${kind}_import_template`)}
            >
              {t('import.template')}
            </Button>
            {/* beforeUpload 返回 false：antd 不自己发请求（要带 Bearer 头、字段名也要对） */}
            <Upload
              accept=".csv,text/csv"
              maxCount={1}
              showUploadList={false}
              beforeUpload={(f) => {
                setFile(f);
                setReport(null);
                return false;
              }}
            >
              <Button icon={<FileTextOutlined />}>{file ? file.name : t('import.choose')}</Button>
            </Upload>
            <Button type="primary" loading={loading} disabled={!file} onClick={() => void submit()}>
              {t('import.upload')}
            </Button>
          </Space>
          {report && (
            <Table<ImportRow>
              rowKey="line"
              size="small"
              columns={columns}
              dataSource={report.rows}
              pagination={false}
              scroll={{ y: 320 }}
            />
          )}
        </Space>
      </Modal>
    </>
  );
}
