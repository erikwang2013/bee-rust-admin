/**
 * 审计变更摘要（项 8）的展示：后端 `audit_log.changes` 是
 * `{"字段":{"old":…,"new":…}}`（密文列是 `{"changed":true}`）的 JSON 串，
 * 这里转成一行「字段: 旧 → 新」的文本（纯逻辑，`auditdiff.test.ts` 覆盖）。
 * 表格里塞原始 JSON 会撑爆行高，v1 就一行文本（不做字段级 diff 组件）。
 */
import type { I18nKey } from './i18n';

/** 后端字段名 → 前端标签键（其余键直接显示原名，别吞掉信息）。 */
const FIELD_KEYS: Record<string, I18nKey> = {
  nickname: 'field.nickname',
  email: 'field.email',
  phone: 'field.phone',
  sex: 'field.sex',
  dept_id: 'field.dept',
  status: 'field.status',
  remark: 'field.remark',
  name: 'field.name',
  code: 'field.code',
  sort: 'field.sort',
  data_scope: 'field.data_scope',
  parent_id: 'field.parent_dept',
  leader: 'field.leader',
  label: 'field.label',
  value: 'field.value',
};

function fmt(v: unknown): string {
  return v === undefined || v === null ? '' : String(v);
}

/** 变更摘要 → 一行文本；空/坏 JSON 原样回显（坏数据也要看得见，不能静默变空）。 */
export function changesText(raw: string | undefined, t: (k: I18nKey) => string): string {
  if (!raw) return '';
  let obj: unknown;
  try {
    obj = JSON.parse(raw);
  } catch {
    return raw;
  }
  if (obj === null || typeof obj !== 'object' || Array.isArray(obj)) return raw;
  return Object.entries(obj as Record<string, unknown>)
    .map(([field, v]) => {
      const label = FIELD_KEYS[field] ? t(FIELD_KEYS[field]) : field;
      const e = (v ?? {}) as { old?: unknown; new?: unknown; changed?: boolean };
      if (e.changed) return `${label}: ${t('auditlog.changed')}`;
      return `${label}: ${fmt(e.old)} → ${fmt(e.new)}`;
    })
    .join('; ');
}
