import type { I18nKey } from './zh-CN';
import { common } from './dict/en/common';
import { fields } from './dict/en/fields';
import { login } from './dict/en/login';
import { shell } from './dict/en/shell';
import { profile } from './dict/en/profile';
import { content } from './dict/en/content';
import { system } from './dict/en/system';
import { ops } from './dict/en/ops';
import { errors } from './dict/en/errors';

/**
 * 英文表。类型写死成 `Record<I18nKey, string>`：中文表加键而这里漏了，
 * `tsc -b` 直接报错（多余键由 i18n.test.ts 运行时再挡一次，两边键集合必须完全一致）。
 *
 * 词表按模块拆在 `dict/en/*.ts`，这里只负责按与中文相同的顺序合并。
 */
export const enUS: Record<I18nKey, string> = {
  ...common,
  ...fields,
  ...login,
  ...shell,
  ...profile,
  ...content,
  ...system,
  ...ops,
  ...errors,
};
