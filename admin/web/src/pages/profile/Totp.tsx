import { useCallback, useEffect, useState } from 'react';
import { Alert, App, Button, Input, Modal, Popconfirm, Space, Table, Tag, Typography } from 'antd';
import type { ColumnsType } from 'antd/es/table';
import { totpApi } from '../../api/auth';
import type { LoginDevice, TotpSetup } from '../../api/types';
import { useAuth } from '../../auth/AuthContext';
import { useI18n } from '../../i18n';

/**
 * 动态验证码 + 登录设备（项 7）。自服务：只管当前登录者自己的东西，所以不需要权限码。
 * 绑定流程是两段式 —— 先生成密钥（回一次性明文 + 二维码），再用 App 上的 6 位码确认；
 * 确认之前 `totp_enabled` 一直是 0，登录不会要码（密钥已经进库，等着被确认）。
 */
export default function Totp() {
  const { user, reload } = useAuth();
  const { message } = App.useApp();
  const { t } = useI18n();
  const [setup, setSetup] = useState<TotpSetup | null>(null);
  const [code, setCode] = useState('');
  const [busy, setBusy] = useState(false);
  const [devices, setDevices] = useState<LoginDevice[]>([]);
  const [backupOpen, setBackupOpen] = useState(false);
  const [backupCode, setBackupCode] = useState('');
  const [backupCodes, setBackupCodes] = useState<string[]>([]);
  const [backupBusy, setBackupBusy] = useState(false);
  const bound = user?.totp_enabled === true;
  const backupLeft = user?.backup_codes_left ?? 0;

  const loadDevices = useCallback(async () => {
    setDevices(await totpApi.devices());
  }, []);

  useEffect(() => {
    void loadDevices();
  }, [loadDevices]);

  const onSetup = async () => {
    setBusy(true);
    try {
      setSetup(await totpApi.setup());
      setCode('');
    } finally {
      setBusy(false);
    }
  };

  const onEnable = async () => {
    setBusy(true);
    try {
      await totpApi.enable(code);
      await reload();
      setSetup(null); // 明文密钥用完就丢，别留在内存/页面上
      setCode('');
      message.success(t('totp.enabled_msg'));
    } finally {
      setBusy(false);
    }
  };

  const onDisable = async () => {
    setBusy(true);
    try {
      await totpApi.disable(code);
      await reload();
      setCode('');
      message.success(t('totp.disabled_msg'));
    } finally {
      setBusy(false);
    }
  };

  const openBackup = () => {
    setBackupCode('');
    setBackupCodes([]);
    setBackupOpen(true);
  };

  const onGenerateBackup = async () => {
    setBackupBusy(true);
    try {
      setBackupCodes((await totpApi.backupCodes(backupCode)).codes);
      setBackupCode('');
      message.success(t('totp.backup_generated'));
    } finally {
      setBackupBusy(false);
    }
  };

  /** 关掉弹窗 = 用户确认「已保存」：明文只在这里出现过，关掉就没有第二次机会 */
  const onBackupSaved = async () => {
    setBackupOpen(false);
    setBackupCodes([]);
    await reload();
  };

  const onKick = async (id: string) => {
    await totpApi.kick(id);
    await loadDevices();
    message.success(t('devices.kicked'));
  };

  const columns: ColumnsType<LoginDevice> = [
    {
      title: t('field.device'), dataIndex: 'user_agent', ellipsis: true,
      render: (ua: string, row) => (
        <Space size={6}>
          <span>{ua || t('devices.unknown')}</span>
          {row.current && <Tag color="blue">{t('devices.current')}</Tag>}
        </Space>
      ),
    },
    { title: t('field.ip'), dataIndex: 'ip', width: 140 },
    { title: t('field.last_active_at'), dataIndex: 'last_active_at', width: 170 },
    {
      title: t('common.actions'), width: 90,
      render: (_, row) => (
        <Popconfirm
          title={t('devices.kick_confirm')}
          okText={t('common.confirm')}
          cancelText={t('common.cancel')}
          okButtonProps={{ danger: true }}
          // 踢自己这台 = 把自己登出，也能用（但别让人误点：这里直接不给按钮）
          disabled={row.current}
          onConfirm={() => void onKick(row.id)}
        >
          <Button danger size="small" disabled={row.current}>{t('devices.kick')}</Button>
        </Popconfirm>
      ),
    },
  ];

  return (
    <>
      <Typography.Title level={5} style={{ marginTop: 0 }}>{t('totp.title')}</Typography.Title>
      <Typography.Paragraph type="secondary" style={{ marginBottom: 12 }}>
        {bound ? t('totp.bound_hint') : t('totp.unbound_hint')}
      </Typography.Paragraph>

      {setup && !bound && (
        <div style={{ marginBottom: 12 }}>
          {/* 二维码是后端出的 SVG：data: URI 内联即可，不用再引 base64 库 */}
          <img
            src={`data:image/svg+xml;utf8,${encodeURIComponent(setup.qr_svg)}`}
            width={180}
            height={180}
            alt={t('totp.qr_alt')}
          />
          <div style={{ fontSize: 12 }}>
            {t('totp.secret')}：<Typography.Text code copyable>{setup.secret}</Typography.Text>
          </div>
        </div>
      )}

      <Space wrap>
        <Input
          value={code}
          onChange={(e) => setCode(e.target.value)}
          placeholder={t('totp.code_placeholder')}
          maxLength={6}
          style={{ width: 160 }}
          autoComplete="one-time-code"
        />
        {!bound && (
          <Button onClick={() => void onSetup()} loading={busy}>
            {setup ? t('totp.regenerate') : t('totp.setup')}
          </Button>
        )}
        {bound ? (
          <Popconfirm
            title={t('totp.disable_confirm')}
            okText={t('common.confirm')}
            cancelText={t('common.cancel')}
            okButtonProps={{ danger: true }}
            onConfirm={() => void onDisable()}
          >
            <Button danger loading={busy}>{t('totp.disable')}</Button>
          </Popconfirm>
        ) : (
          <Button type="primary" disabled={!code} loading={busy} onClick={() => void onEnable()}>
            {t('totp.enable')}
          </Button>
        )}
      </Space>
      <div style={{ marginTop: 6, color: 'rgba(128,128,128,1)', fontSize: 12 }}>{t('totp.no_backup_hint')}</div>

      {bound && (
        <Space style={{ marginTop: 12 }} wrap>
          <span>{t('totp.backup_left', { n: backupLeft })}</span>
          <Button size="small" onClick={openBackup}>
            {backupLeft > 0 ? t('totp.backup_regenerate') : t('totp.backup_generate')}
          </Button>
        </Space>
      )}

      <Modal
        open={backupOpen}
        title={t('totp.backup_title')}
        maskClosable={false}
        keyboard={false}
        closable={backupCodes.length === 0}
        onCancel={() => setBackupOpen(false)}
        footer={
          backupCodes.length > 0
            ? [
                <Button key="saved" type="primary" onClick={() => void onBackupSaved()}>
                  {t('totp.backup_done')}
                </Button>,
              ]
            : [
                <Button key="cancel" onClick={() => setBackupOpen(false)}>{t('common.cancel')}</Button>,
                <Button
                  key="generate"
                  type="primary"
                  loading={backupBusy}
                  disabled={backupCode.length !== 6}
                  onClick={() => void onGenerateBackup()}
                >
                  {t('totp.backup_generate')}
                </Button>,
              ]
        }
      >
        {backupCodes.length === 0 ? (
          <>
            <Typography.Paragraph type="secondary">{t('totp.backup_prompt')}</Typography.Paragraph>
            <Input
              value={backupCode}
              onChange={(e) => setBackupCode(e.target.value)}
              placeholder={t('totp.code_placeholder')}
              maxLength={6}
              autoComplete="one-time-code"
            />
          </>
        ) : (
          <>
            <Alert type="warning" showIcon message={t('totp.backup_once')} />
            <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 6, marginTop: 12, fontFamily: 'monospace' }}>
              {backupCodes.map((c) => <span key={c}>{c}</span>)}
            </div>
          </>
        )}
      </Modal>

      <Typography.Title level={5} style={{ marginTop: 24 }}>{t('devices.title')}</Typography.Title>
      <Table<LoginDevice>
        rowKey="id"
        size="small"
        columns={columns}
        dataSource={devices}
        pagination={false}
      />
    </>
  );
}
