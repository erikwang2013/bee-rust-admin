import axios, { type AxiosRequestConfig } from 'axios';
import { message } from 'antd';
import type { ApiResult } from './types';

export const TOKEN_KEY = 'bee_admin_token';

const client = axios.create({ baseURL: '/api/v1', timeout: 15000 });

client.interceptors.request.use((cfg) => {
  const token = localStorage.getItem(TOKEN_KEY);
  if (token) cfg.headers.Authorization = `Bearer ${token}`;
  return cfg;
});

/**
 * 业务信封判定：HTTP 200 也可能是失败（`code != 0`）。返回错误文案，成功返回 null。
 *
 * 单独抽出来是为了能在没有网络的情况下测——拦截器只是拿它决定 reject 还是放行。
 * 注意放行条件严格：非对象、没有数字 code 的一律当成功（blob 等非信封响应）。
 */
export function envelopeError(body: unknown): string | null {
  if (!body || typeof body !== 'object') return null;
  const { code, msg } = body as ApiResult<unknown>;
  return typeof code === 'number' && code !== 0 ? msg || '请求失败' : null;
}

client.interceptors.response.use(
  (res) => {
    const msg = envelopeError(res.data);
    if (msg) {
      message.error(msg);
      return Promise.reject(new Error(msg));
    }
    return res;
  },
  (err) => {
    const status = err.response?.status;
    if (status === 401) {
      localStorage.removeItem(TOKEN_KEY);
      if (!window.location.pathname.startsWith('/login')) {
        window.location.href = '/login';
      }
    } else if (status === 403) {
      message.error(err.response?.data?.msg || '没有权限');
    } else {
      message.error(err.response?.data?.msg || err.message || '网络错误');
    }
    return Promise.reject(err);
  },
);

async function unwrap<T>(p: Promise<{ data: ApiResult<T> }>): Promise<T> {
  const res = await p;
  return res.data.data;
}

export const http = {
  get: <T>(url: string, params?: object, cfg?: AxiosRequestConfig) =>
    unwrap<T>(client.get(url, { params, ...cfg })),
  post: <T>(url: string, data?: object) => unwrap<T>(client.post(url, data)),
  put: <T>(url: string, data?: object) => unwrap<T>(client.put(url, data)),
  del: <T>(url: string, params?: object) => unwrap<T>(client.delete(url, { params })),
  /** 导出等二进制响应：不走 unwrap（响应体不是 JSON 信封）。 */
  blob: (url: string, params?: object) => client.get<Blob>(url, { params, responseType: 'blob' }),
};
