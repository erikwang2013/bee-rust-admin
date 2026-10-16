import axios, { type AxiosRequestConfig } from 'axios';
import { message } from 'antd';
import { errText, t } from '../i18n';
import type { ApiResult } from './types';

export const TOKEN_KEY = 'bee_admin_token';

declare module 'axios' {
  /**
   * 请求级开关：`silent: true` 的那一条失败不弹全局 toast。
   * **只给「拿不到就走兜底」的旁路读取用**（目前仅公开配置一处）；业务请求一律不传 ——
   * 弹红字是这层拦截器的职责，随手静默会把真错误一起吞掉。
   */
  export interface AxiosRequestConfig {
    silent?: boolean;
  }
}

/** 导出只为测试能换掉 `defaults.adapter`（见 client.test.ts 的拦截器用例）；业务代码用 `http`。 */
export const client = axios.create({ baseURL: '/api/v1', timeout: 15000 });

client.interceptors.request.use((cfg) => {
  const token = localStorage.getItem(TOKEN_KEY);
  if (token) cfg.headers.Authorization = `Bearer ${token}`;
  return cfg;
});

/**
 * 后端给的错误文案 → 界面文案。C3 契约：信封里有 `err` 且词表里有 `err.<code>`
 * 就用本地化文案（`args` 做占位替换，`field` 查 `field.*` 标签）；
 * 否则回落后端的中文 `msg`。**任何情况都有话可说**。
 */
function backendError(data: unknown, fallback: string): string {
  if (data && typeof data === 'object') {
    const { msg, err, args } = data as ApiResult<unknown>;
    const mapped = typeof err === 'string' ? errText(err, args) : null;
    if (mapped || msg) return mapped || msg;
  }
  return fallback;
}

/**
 * 业务信封判定：HTTP 200 也可能是失败（`code != 0`）。返回错误文案，成功返回 null。
 *
 * 单独抽出来是为了能在没有网络的情况下测——拦截器只是拿它决定 reject 还是放行。
 * 注意放行条件严格：非对象、没有数字 code 的一律当成功（blob 等非信封响应）。
 */
export function envelopeError(body: unknown): string | null {
  if (!body || typeof body !== 'object') return null;
  const { code } = body as ApiResult<unknown>;
  return typeof code === 'number' && code !== 0 ? backendError(body, t('common.request_failed')) : null;
}

client.interceptors.response.use(
  (res) => {
    const msg = envelopeError(res.data);
    if (msg) {
      if (!res.config.silent) message.error(msg);
      return Promise.reject(new Error(msg));
    }
    return res;
  },
  (err) => {
    const status = err.response?.status;
    // silent 只压 toast（见 AxiosRequestConfig.silent）：401 的清 token / 跳登录是登录语义，
    // 静默请求（公开配置是免登录接口）也照走，免得缺一个「静默就绕过登录」的口子。
    const silent = err.config?.silent === true;
    if (status === 401) {
      localStorage.removeItem(TOKEN_KEY);
      if (!window.location.pathname.startsWith('/login')) {
        window.location.href = '/login';
      }
    } else if (status === 403) {
      if (!silent) message.error(backendError(err.response?.data, t('common.no_permission')));
    } else if (!silent) {
      message.error(backendError(err.response?.data, err.message || t('common.network_error')));
    }
    return Promise.reject(err);
  },
);

/**
 * 从请求失败里取后端信封的错误码。**只给需要按码分支的调用方用**（登录页要靠
 * `auth.totp_required` 决定是否显示动态验证码框）；只是提示文案的话交给拦截器。
 */
export function errCodeOf(e: unknown): string | undefined {
  const data = (e as { response?: { data?: ApiResult<unknown> } })?.response?.data;
  return typeof data?.err === 'string' ? data.err : undefined;
}

async function unwrap<T>(p: Promise<{ data: ApiResult<T> }>): Promise<T> {
  const res = await p;
  return res.data.data;
}

export const http = {
  /** `cfg` 目前只有 `silent` 在用（见 `AxiosRequestConfig.silent`），别当透传口子用。 */
  get: <T>(url: string, params?: object, cfg?: AxiosRequestConfig) =>
    unwrap<T>(client.get(url, { params, ...cfg })),
  post: <T>(url: string, data?: object) => unwrap<T>(client.post(url, data)),
  put: <T>(url: string, data?: object) => unwrap<T>(client.put(url, data)),
  del: <T>(url: string, params?: object) => unwrap<T>(client.delete(url, { params })),
  /** 导出等二进制响应：不走 unwrap（响应体不是 JSON 信封）。 */
  blob: (url: string, params?: object) => client.get<Blob>(url, { params, responseType: 'blob' }),
};
