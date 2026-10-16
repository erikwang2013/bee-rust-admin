import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { AxiosError, type AxiosResponse, type InternalAxiosRequestConfig } from 'axios';

// 全局 toast 换成可断言的桩。`client.ts` 里 `import { message } from 'antd'` 拿到的是同一个
// 对象，所以这一层断言的就是「拦截器到底弹没弹」。
const { toast } = vi.hoisted(() => ({ toast: vi.fn() }));
vi.mock('antd', () => ({ message: { error: toast } }));

import { TOKEN_KEY, client, envelopeError, errCodeOf, http } from './client';

describe('errCodeOf', () => {
  it('从 axios 错误里取信封错误码（登录页靠它判断要不要亮动态验证码框）', () => {
    expect(errCodeOf({ response: { data: { code: 400, msg: '需要动态验证码', err: 'auth.totp_required' } } }))
      .toBe('auth.totp_required');
  });

  it('没有响应体 / 不是信封 / 没有 err 一律 undefined，不能凭空造一个码出来', () => {
    expect(errCodeOf(new Error('网络错误'))).toBeUndefined();
    expect(errCodeOf({ response: { data: 'plain' } })).toBeUndefined();
    expect(errCodeOf({ response: { data: { code: 400, msg: '没有码' } } })).toBeUndefined();
    expect(errCodeOf(null)).toBeUndefined();
  });
});

describe('envelopeError', () => {
  it('code != 0 判为业务失败，带出后端消息', () => {
    expect(envelopeError({ code: 40301, msg: '没有权限', data: null })).toBe('没有权限');
  });

  it('code != 0 且 msg 为空时给默认文案', () => {
    expect(envelopeError({ code: 500, msg: '' })).toBe('请求失败');
  });

  it('code == 0 放行', () => {
    expect(envelopeError({ code: 0, msg: 'ok', data: { id: 1 } })).toBeNull();
  });

  it('非信封响应（blob / null / 没有数字 code）放行，不能误判成失败', () => {
    expect(envelopeError(new Blob(['id,name\n1,a\n']))).toBeNull();
    expect(envelopeError(null)).toBeNull();
    expect(envelopeError(undefined)).toBeNull();
    expect(envelopeError('plain text')).toBeNull();
    expect(envelopeError({ code: '0', msg: 'ok' })).toBeNull();
  });

  it('有 err 且词表命中：用词表文案（带 args 占位）', () => {
    expect(envelopeError({ code: 400, msg: '用户名或密码错误', err: 'auth.bad_credentials' }))
      .toBe('用户名或密码错误');
    expect(envelopeError({
      code: 400, msg: '备注长度不能超过 255 个字符',
      err: 'common.too_long', args: { field: 'remark', max: 255 },
    })).toBe('备注 长度不能超过 255 个字符');
  });

  it('err 不在词表里：回落 msg，不能吞掉后端的话', () => {
    expect(envelopeError({ code: 400, msg: '这个码前端还没见过', err: 'brand.new.code' }))
      .toBe('这个码前端还没见过');
    expect(envelopeError({ code: 500, msg: '', err: 'brand.new.code' })).toBe('请求失败');
  });
});

// ── 拦截器 · silent（公开配置那一处）─────────────────────────
//
// 换掉 adapter 直接喂响应，让请求真的走一遍 axios 的合并与两个拦截器 ——
// 这样连「自定义的 config 键会不会被 axios 丢掉」也一并钉住（丢掉的话 silent 就是哑的）。
const envelope = (body: unknown, status = 200) =>
  async (cfg: InternalAxiosRequestConfig): Promise<AxiosResponse> =>
    ({ data: body, status, statusText: 'OK', headers: {}, config: cfg });

const bomb = (status?: number) =>
  (cfg: InternalAxiosRequestConfig): Promise<never> =>
    Promise.reject(new AxiosError('boom', 'ERR_NETWORK', cfg, undefined, status
      ? { data: { code: status, msg: '后端炸了' }, status, statusText: '', headers: {}, config: cfg }
      : undefined));

describe('响应拦截器 · silent', () => {
  beforeEach(() => {
    toast.mockClear();
    // 请求拦截器要读 token；node 环境没有 localStorage（401 那条用例会换成自己的桩）
    vi.stubGlobal('localStorage', { getItem: () => null, removeItem: vi.fn() });
  });
  afterEach(() => vi.unstubAllGlobals());

  it('默认（业务请求）：信封失败照弹全局 toast —— silent 不许把全局错误处理削弱掉', async () => {
    client.defaults.adapter = envelope({ code: 400, msg: '业务失败' });
    await expect(http.get('/x')).rejects.toThrow('业务失败');
    expect(toast).toHaveBeenCalledWith('业务失败');
  });

  it('silent：同样的失败不弹，但照样 reject（拿不到由调用方自己兜底）', async () => {
    client.defaults.adapter = envelope({ code: 400, msg: '业务失败' });
    await expect(http.get('/x', undefined, { silent: true })).rejects.toThrow('业务失败');
    expect(toast).not.toHaveBeenCalled();
  });

  it('silent：连不上后端（没响应体）也不弹 —— 这正是登录页「后端挂了」的那条红字', async () => {
    client.defaults.adapter = bomb();
    await expect(http.get('/x', undefined, { silent: true })).rejects.toThrow('boom');
    expect(toast).not.toHaveBeenCalled();
  });

  it('不是 silent 的网络错误照样弹（同一个 bomb，差别只在请求选项）', async () => {
    client.defaults.adapter = bomb();
    await expect(http.get('/x')).rejects.toThrow('boom');
    expect(toast).toHaveBeenCalledTimes(1);
  });

  it('silent 只压 toast，不动成功路径', async () => {
    client.defaults.adapter = envelope({ code: 0, msg: 'ok', data: { 'site.copyright': 'x' } });
    await expect(http.get('/x', undefined, { silent: true })).resolves.toEqual({ 'site.copyright': 'x' });
    expect(toast).not.toHaveBeenCalled();
  });

  it('silent 只压 toast：401 清 token / 跳登录照旧（静默不等于哑）', async () => {
    const loc = { pathname: '/system/admin', href: '' };
    const removeItem = vi.fn();
    vi.stubGlobal('localStorage', { getItem: () => 'tok', removeItem });
    vi.stubGlobal('window', { location: loc });
    client.defaults.adapter = bomb(401);
    await expect(http.get('/x', undefined, { silent: true })).rejects.toThrow();
    expect(removeItem).toHaveBeenCalledWith(TOKEN_KEY);
    expect(loc.href).toBe('/login');
    expect(toast).not.toHaveBeenCalled();
  });
});
