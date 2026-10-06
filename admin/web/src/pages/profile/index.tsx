import { useRef, useState, type ChangeEvent } from 'react';
import { App, Avatar, Button, Card, Divider, Form, Input, Popconfirm, Space } from 'antd';
import { UploadOutlined } from '@ant-design/icons';
import { authApi } from '../../api/auth';
import { TOKEN_KEY } from '../../api/client';
import { avatarUrl, useAuth } from '../../auth/AuthContext';
import { useI18n } from '../../i18n';

const MAX_DATA_URL = 512 * 1024; // 后端对 data_url 与解码后字节双重卡 512KB

/** 压到极限仍超 512KB：调用方要单独提示，别和「读不出来」混成一句。 */
class AvatarTooLarge extends Error {}

/** 缩到最长边 512px，再降 JPEG 质量，直到 data_url 不超过 512KB。 */
async function compressToDataUrl(file: File): Promise<string> {
  const bitmap = await createImageBitmap(file);
  const scale = Math.min(1, 512 / Math.max(bitmap.width, bitmap.height));
  const canvas = document.createElement('canvas');
  canvas.width = Math.max(1, Math.round(bitmap.width * scale));
  canvas.height = Math.max(1, Math.round(bitmap.height * scale));
  const ctx = canvas.getContext('2d');
  if (!ctx) {
    bitmap.close();
    throw new Error('canvas 不可用');
  }
  ctx.fillStyle = '#fff'; // PNG 透明底转 JPEG 会变黑，先铺白
  ctx.fillRect(0, 0, canvas.width, canvas.height);
  ctx.drawImage(bitmap, 0, 0, canvas.width, canvas.height);
  bitmap.close();

  let quality = 0.85;
  let url = canvas.toDataURL('image/jpeg', quality);
  while (url.length > MAX_DATA_URL && quality > 0.3) {
    quality = Math.max(0.3, quality - 0.15);
    url = canvas.toDataURL('image/jpeg', quality);
  }
  if (url.length > MAX_DATA_URL) throw new AvatarTooLarge();
  return url;
}

export default function ProfilePage() {
  const { user, logout, reload, version } = useAuth();
  const { message } = App.useApp();
  const { t } = useI18n();
  const [pwdForm] = Form.useForm();
  const [infoForm] = Form.useForm();
  const fileRef = useRef<HTMLInputElement>(null);
  const [savingInfo, setSavingInfo] = useState(false);
  const [savingPwd, setSavingPwd] = useState(false);
  const [uploading, setUploading] = useState(false);
  const [kicking, setKicking] = useState(false);

  const onPickAvatar = async (e: ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    e.target.value = ''; // 允许重复选同一个文件
    if (!file) return;
    let data_url: string;
    try {
      data_url = await compressToDataUrl(file);
    } catch (e) {
      // canvas 不可用等环境问题也走这里，统一按「读不出来」提示
      message.error(t(e instanceof AvatarTooLarge ? 'profile.avatar_too_large' : 'profile.avatar_failed'));
      return;
    }
    setUploading(true);
    try {
      await authApi.uploadAvatar(data_url);
      await reload();
      message.success(t('profile.avatar_updated'));
    } finally {
      setUploading(false);
    }
  };

  const onSaveInfo = async (v: { nickname: string; email?: string; phone?: string }) => {
    setSavingInfo(true);
    try {
      await authApi.updateProfile({
        nickname: v.nickname,
        email: v.email ?? '',
        phone: v.phone ?? '',
      });
      await reload();
      message.success(t('profile.info_saved'));
    } finally {
      setSavingInfo(false);
    }
  };

  const onChangePwd = async (v: { old_password: string; new_password: string }) => {
    setSavingPwd(true);
    try {
      await authApi.changePassword(v.old_password, v.new_password);
      message.success(t('profile.pwd_changed'));
      pwdForm.resetFields();
      await logout();
      window.location.href = '/login';
    } finally {
      setSavingPwd(false);
    }
  };

  /**
   * 退出其他设备：后端把 token_version +1，别的设备手里的 token 全失效；
   * 当前设备同时拿到一个新 token，所以本机不掉线。
   */
  const onLogoutOthers = async () => {
    setKicking(true);
    try {
      const { token } = await authApi.logoutOthers();
      localStorage.setItem(TOKEN_KEY, token); // 先落新 token，reload 才能带着它拉 profile
      await reload();
      message.success(t('profile.kicked'));
    } finally {
      setKicking(false);
    }
  };

  return (
    <Card title={t('profile.title')} style={{ maxWidth: 640 }}>
      <Space size={16} align="center" style={{ marginBottom: 24 }}>
        <Avatar
          size={64}
          src={avatarUrl(user?.avatar, version)}
          icon={<img src="/keeper-head.svg" alt={t('common.mascot_alt')} width={52} height={52} />}
        />
        <div>
          <input
            ref={fileRef}
            type="file"
            accept="image/png,image/jpeg"
            style={{ display: 'none' }}
            onChange={(e) => void onPickAvatar(e)}
          />
          <Button icon={<UploadOutlined />} loading={uploading} onClick={() => fileRef.current?.click()}>
            {t('profile.change_avatar')}
          </Button>
          <div style={{ marginTop: 4, color: 'rgba(128,128,128,1)', fontSize: 12 }}>
            {t('profile.avatar_hint')}
          </div>
        </div>
      </Space>

      <Form
        form={infoForm}
        layout="vertical"
        style={{ maxWidth: 360 }}
        initialValues={{ nickname: user?.nickname, email: user?.email, phone: user?.phone }}
        onFinish={(v) => void onSaveInfo(v)}
      >
        <Form.Item
          name="nickname"
          label={t('field.nickname')}
          rules={[{ required: true, max: 64, message: t('validate.required_max', { field: t('field.nickname'), max: 64 }) }]}
        >
          <Input />
        </Form.Item>
        <Form.Item name="email" label={t('field.email')} rules={[{ type: 'email', message: t('validate.email') }]}>
          <Input />
        </Form.Item>
        <Form.Item name="phone" label={t('field.phone')}>
          <Input />
        </Form.Item>
        <Button type="primary" htmlType="submit" loading={savingInfo}>{t('profile.save_info')}</Button>
      </Form>

      <Divider />

      <Form
        form={pwdForm}
        layout="vertical"
        style={{ maxWidth: 360 }}
        onFinish={(v) => void onChangePwd(v)}
      >
        <Form.Item
          name="old_password"
          label={t('field.old_password')}
          rules={[{ required: true, message: t('validate.required', { field: t('field.old_password') }) }]}
        >
          <Input.Password autoComplete="current-password" />
        </Form.Item>
        <Form.Item
          name="new_password"
          label={t('field.new_password')}
          rules={[{ required: true, min: 6, message: t('validate.min_len', { field: t('field.new_password'), n: 6 }) }]}
        >
          <Input.Password autoComplete="new-password" />
        </Form.Item>
        <Form.Item
          name="confirm"
          label={t('field.confirm_password')}
          dependencies={['new_password']}
          rules={[
            { required: true, message: t('profile.confirm_required') },
            ({ getFieldValue }) => ({
              validator: (_, value) =>
                !value || getFieldValue('new_password') === value
                  ? Promise.resolve()
                  : Promise.reject(new Error(t('validate.mismatch'))),
            }),
          ]}
        >
          <Input.Password autoComplete="new-password" />
        </Form.Item>
        <Button type="primary" htmlType="submit" loading={savingPwd}>{t('profile.change_pwd')}</Button>
      </Form>

      <Divider />

      <Popconfirm
        title={t('profile.kick_others')}
        description={t('profile.kick_confirm')}
        okText={t('profile.kick_ok')}
        cancelText={t('common.cancel')}
        okButtonProps={{ danger: true }}
        onConfirm={() => void onLogoutOthers()}
      >
        <Button danger loading={kicking}>{t('profile.kick_others')}</Button>
      </Popconfirm>
    </Card>
  );
}
