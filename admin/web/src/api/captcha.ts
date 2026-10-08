import { http } from './client';
import type { LoginResult } from './auth';
import type { CaptchaAnswer, CaptchaData } from './types';

export const captchaApi = {
  /**
   * 取一张验证码（`GET /api/v1/captcha/new`，公开接口）。
   * 后端 `[auth] captcha = false` 时 `data` 为 null —— 前端据此不渲染验证码区，
   * 不需要另开一个「查配置」接口，也不会出现「显示了验证码但后端根本不校验」的错位。
   */
  create: () => http.get<CaptchaData | null>('/captcha/new'),
};

/**
 * 登录（全站唯一的登录入口）。验证码凭据**随登录体一起提交** —— 校验发生在
 * 登录接口内部（后端 `verify_captcha`），不是「前端先验一次再登录」，
 * 那样两步之间会有插空窗口。`[auth] captcha = false` 时这两个字段不传。
 * `captcha_answer` 是插件的 `Answer` 表示，见 `pages/login/captcha.ts` 里的构造。
 */
export function loginWithCaptcha(body: {
  username: string;
  password: string;
  /** 验证码关闭 / 未取到验证码时不带这两个字段 */
  captcha_key?: string;
  captcha_answer?: CaptchaAnswer;
}) {
  return http.post<LoginResult>('/auth/login', body);
}
