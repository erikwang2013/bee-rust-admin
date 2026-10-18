// 中文基准词表（模块化拆分；由 ../../zh-CN.ts 合并导出）。
export const login = {
  // ---- 登录页 ----
  'login.title': 'BRA · 管理后台',
  'login.submit': '登录',
  // 验证码区（poster-rust 三种类型随机切，提示文案跟着类型走）
  'login.captcha_click_hint': '按顺序点击图中出现的文字/图案',
  'login.captcha_rotate_hint': '拖动图片，把它转正',
  'login.captcha_slider_hint': '拖动拼图块，补上图中的缺口',
  'login.captcha_refresh': '换一张',
  'login.captcha_undo': '撤销',
  'login.captcha_required': '请先完成验证码',
  'login.code_required': '请填写动态验证码',
  'login.code_hint': '该账号已开启动态验证码：请填验证器 App 上的 6 位码，或用一枚备用码登录',
} as const;
