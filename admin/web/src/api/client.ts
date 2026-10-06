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

client.interceptors.response.use(
  (res) => {
    const body = res.data as ApiResult<unknown>;
    if (body && typeof body.code === 'number' && body.code !== 0) {
      message.error(body.msg || '请求失败');
      return Promise.reject(new Error(body.msg || 'request failed'));
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
