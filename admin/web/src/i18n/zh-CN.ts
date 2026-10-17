/**
 * 中文是**基准词表**：所有键从这里推导（`I18nKey`），英文表写成 `Record<I18nKey, string>`
 * 且是 `as const` 之外的显式 Record —— 漏一个键就编译不过（见 i18n.test.ts 的另一半保障）。
 *
 * 命名约定：
 * - `field.*`   字段标签。**同时是后端 `args.field` 的查表来源**（契约：
 *               `check_len("remark", …)` 带 `field:"remark"`，前端查 `field.remark`），
 *               所以键名要与后端传的字段键逐字一致，缺了只会原样显示键名，不会白屏。
 * - `err.*`     后端业务错误码，键名 = 错误码本身（`err.auth.bad_credentials`）。
 *               文案与后端 msg 的中文原文一致；`args` 用 `{name}` 占位。
 * - 其余按模块分（login / layout / admin / role / …），`common.*` 是跨页共用文案。
 *
 * **不做**数据库里的业务数据（菜单名、角色名、公告内容）——那是内容，不是界面文案。
 *
 * 词表按模块拆在 `dict/zh/*.ts`，这里只负责按原顺序合并；键集合与文案的单一事实来源
 * 仍是这些模块文件（i18n.test.ts 会校验中英键集合一致）。
 */
import { common } from './dict/zh/common';
import { fields } from './dict/zh/fields';
import { login } from './dict/zh/login';
import { shell } from './dict/zh/shell';
import { profile } from './dict/zh/profile';
import { content } from './dict/zh/content';
import { system } from './dict/zh/system';
import { ops } from './dict/zh/ops';
import { errors } from './dict/zh/errors';

export const zhCN = {
  ...common,
  ...fields,
  ...login,
  ...shell,
  ...profile,
  ...content,
  ...system,
  ...ops,
  ...errors,
} as const;

export type I18nKey = keyof typeof zhCN;
