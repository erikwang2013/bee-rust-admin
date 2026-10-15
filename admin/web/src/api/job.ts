import { http } from './client';
import type { Job, JobLog, JobRunResult, Page } from './types';

export interface JobQuery { page?: number; size?: number; name?: string; status?: number }
/** name 与 code 是代码注册键，不可改 —— 更新体里只有间隔与状态；间隔在协议上是字符串 */
export interface JobForm { cron: string; status: number }
export interface JobLogQuery {
  page?: number; size?: number; job_code?: string; status?: number; start?: string; end?: string;
}

export const jobApi = {
  list: (q: JobQuery) => http.get<Page<Job>>('/jobs', q),
  update: (id: string, data: JobForm) => http.put<null>(`/jobs/${id}`, data),
  /** 后台执行一次（立即返回，不等跑完）；已在执行中 / code 没注册时后端 409（拦截器提示 msg） */
  run: (id: string) => http.post<JobRunResult>(`/jobs/${id}/run`),
  logs: (q: JobLogQuery) => http.get<Page<JobLog>>('/job-logs', q),
};
