import type { I18nKey } from '../../../i18n';

/** 与后端 audit.rs 的 module_action 取值一一对应（未列出的路径归 other）。
 * 单独一份：操作日志页与导出中心（筛选式导出）共用同一份模块清单。 */
export const AUDIT_MODULES: Record<string, I18nKey> = {
  admin: 'auditlog.module.admin',
  role: 'auditlog.module.role',
  menu: 'auditlog.module.menu',
  dept: 'auditlog.module.dept',
  dict: 'auditlog.module.dict',
  config: 'auditlog.module.config',
  import: 'auditlog.module.import',
  export: 'auditlog.module.export',
  loginlog: 'auditlog.module.loginlog',
  auditlog: 'auditlog.module.auditlog',
  attach: 'auditlog.module.attach',
  auth: 'auditlog.module.auth',
  other: 'auditlog.module.other',
};
