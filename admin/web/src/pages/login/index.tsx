import { useCallback, useEffect, useState } from 'react';
import { App, Button, Card, Form, Input, theme, Typography } from 'antd';
import { LockOutlined, UserOutlined } from '@ant-design/icons';
import { useLocation, useNavigate } from 'react-router';
import { useAuth } from '../../auth/AuthContext';
import { ThemeToggle } from '../../theme';
import { LanguageToggle, useI18n } from '../../i18n';
import { TOKEN_KEY } from '../../api/client';
import { captchaApi, loginWithCaptcha } from '../../api/captcha';
import type { CaptchaAnswer, CaptchaData } from '../../api/types';
import CaptchaBox from './CaptchaBox';

export default function LoginPage() {
  const { reload } = useAuth();
  const nav = useNavigate();
  const loc = useLocation();
  const { token } = theme.useToken();
  const { t } = useI18n();
  const { message } = App.useApp();
  const [loading, setLoading] = useState(false);
  /** null = 后端没开验证码（或没取到），此时不渲染验证码区、登录体也不带凭据 */
  const [captcha, setCaptcha] = useState<CaptchaData | null>(null);
  const [answer, setAnswer] = useState<CaptchaAnswer | null>(null);

  const loadCaptcha = useCallback(async () => {
    try {
      setCaptcha(await captchaApi.create());
    } catch {
      // 取不到就当没开：后端开着的话下一次登录会被它挡回来（400 auth.captcha_failed）
      setCaptcha(null);
    }
    setAnswer(null); // 换了题目，上一次的答案作废
  }, []);

  useEffect(() => {
    void loadCaptcha();
  }, [loadCaptcha]);

  const onFinish = async (v: { username: string; password: string }) => {
    if (captcha && !answer) {
      message.error(t('login.captcha_required'));
      return;
    }
    setLoading(true);
    try {
      // 验证码凭据随登录体一起提交，校验在登录接口内部（后端 verify_captcha）
      const res = await loginWithCaptcha({
        username: v.username,
        password: v.password,
        ...(captcha && answer ? { captcha_key: captcha.key, captcha_answer: answer } : {}),
      });
      localStorage.setItem(TOKEN_KEY, res.token); // 先落 token，reload 才能带着它拉 profile
      await reload();
      // 守卫写入的来路（未登录访问的页面），没有则回首页
      const from = (loc.state as { from?: string } | null)?.from;
      nav(from ?? '/', { replace: true });
    } catch {
      // 具体错误消息由 axios 拦截器提示
      // 验证码**一次性**：登录成功才发 token 之外，无论哪种失败都得换一张
      // （答对但密码错时 key 已被服务端消费掉，留着只会让下次必错）
      await loadCaptcha();
    } finally {
      setLoading(false);
    }
  };

  return (
    <div style={{ height: '100vh', display: 'flex', alignItems: 'center', justifyContent: 'center', background: token.colorBgLayout }}>
      <div style={{ position: 'fixed', top: 16, right: 16, display: 'flex', gap: 12 }}>
        <LanguageToggle />
        <ThemeToggle />
      </div>
      <Card style={{ width: 380 }}>
        <div style={{ textAlign: 'center' }}>
          <img src="/keeper.svg" alt={t('common.mascot_alt')} width={132} height={132} />
        </div>
        <Typography.Title level={3} style={{ textAlign: 'center', marginTop: 0, marginBottom: 24 }}>
          {t('login.title')}
        </Typography.Title>
        <Form onFinish={onFinish} size="large">
          <Form.Item name="username" rules={[{ required: true, message: t('validate.required', { field: t('field.username') }) }]}>
            <Input prefix={<UserOutlined />} placeholder={t('field.username')} autoComplete="username" />
          </Form.Item>
          <Form.Item name="password" rules={[{ required: true, message: t('validate.required', { field: t('field.password') }) }]}>
            <Input.Password prefix={<LockOutlined />} placeholder={t('field.password')} autoComplete="current-password" />
          </Form.Item>
          {captcha && (
            <Form.Item>
              {/* key 换掉时重挂：拖过的位置、点过的标记要跟着清零 */}
              <CaptchaBox key={captcha.key} data={captcha} onAnswer={setAnswer} onRefresh={() => void loadCaptcha()} />
            </Form.Item>
          )}
          <Form.Item>
            <Button type="primary" htmlType="submit" block loading={loading}>
              {t('login.submit')}
            </Button>
          </Form.Item>
        </Form>
      </Card>
    </div>
  );
}
