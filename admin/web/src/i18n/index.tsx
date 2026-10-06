import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from 'react';
import { Select } from 'antd';
import { GlobalOutlined } from '@ant-design/icons';
import dayjs from 'dayjs';
import 'dayjs/locale/zh-cn';
import { zhCN, type I18nKey } from './zh-CN';
import { enUS } from './en-US';

export type { I18nKey };

export type Lang = 'zh-CN' | 'en-US';
export type TFunc = typeof t;
type Args = Record<string, string | number>;

const LANG_KEY = 'bee_admin_lang';

const DICTS: Record<Lang, Record<string, string | undefined>> = { 'zh-CN': zhCN, 'en-US': enUS };

/**
 * 语言探测（纯函数，单测覆盖）：`zh*` → zh-CN，其它语言 → en-US。
 * 空串（探测不到 navigator.language）回落中文基准 —— 中文原文是最后一道防线。
 */
export function detectLang(navLang: string): Lang {
  const s = navLang.trim().toLowerCase();
  if (s.startsWith('zh')) return 'zh-CN';
  return s ? 'en-US' : 'zh-CN';
}

/** 记住的偏好优先，否则跟随浏览器。 */
function initialLang(): Lang {
  // vitest 跑在 node 环境（无 window/localStorage），此时用基准语言，测试断言才稳定
  if (typeof window === 'undefined') return 'zh-CN';
  const saved = localStorage.getItem(LANG_KEY);
  if (saved === 'zh-CN' || saved === 'en-US') return saved;
  return detectLang(typeof navigator === 'undefined' ? '' : navigator.language);
}

/**
 * 当前语言。**放在模块级而不是只放 Context**：axios 拦截器、任务页的 format.ts
 * 这类非 React 代码也要能直接 `t()`。Context 只负责「语言变了通知界面重渲染」。
 */
let current: Lang = initialLang();

/** 取文案；`{name}` 占位用 args 替换。**键不存在返回键名本身**（不抛异常、不白屏）。 */
export function t(key: I18nKey, args?: Args): string {
  const s = DICTS[current][key];
  if (s === undefined) return key;
  if (!args) return s;
  return s.replace(/\{(\w+)\}/g, (m, k: string) => (k in args ? String(args[k]) : m));
}

/**
 * 后端业务错误码 → 文案（C3 契约：响应信封里的 `err` + `args`）。
 * 表里没有这个码返回 null，调用方回落 `msg` —— 保证任何情况都有话可说。
 *
 * `args.field` 传的是**字段键**（如 `"remark"`），用 `field.remark` 查标签；
 * 查不到就原样显示键名（宁可露出 remark 也别空着）。
 */
export function errText(code: string, args?: Record<string, unknown>): string | null {
  const key = `err.${code}` as I18nKey;
  if (!(key in zhCN)) return null;
  const a: Args = {};
  for (const [k, v] of Object.entries(args ?? {})) a[k] = typeof v === 'number' ? v : String(v);
  if (typeof args?.field === 'string') {
    const fk = `field.${args.field}` as I18nKey;
    a.field = fk in zhCN ? t(fk) : args.field;
  }
  return t(key, a);
}

const I18nCtx = createContext<{ lang: Lang; setLang: (l: Lang) => void } | null>(null);

export function I18nProvider({ children }: { children: ReactNode }) {
  const [lang, setLangState] = useState<Lang>(current);

  const setLang = useCallback((l: Lang) => {
    current = l;
    localStorage.setItem(LANG_KEY, l);
    setLangState(l);
  }, []);

  // 切换时同步 dayjs 与 <html lang>；antd 的 ConfigProvider locale 在 main.tsx 跟着 lang 传
  useEffect(() => {
    dayjs.locale(lang === 'zh-CN' ? 'zh-cn' : 'en');
    document.documentElement.lang = lang;
  }, [lang]);

  const value = useMemo(() => ({ lang, setLang }), [lang, setLang]);
  return <I18nCtx.Provider value={value}>{children}</I18nCtx.Provider>;
}

/** 组件里取 `t` 的同时订阅语言变化：语言一换，用它的组件必然重渲染。 */
export function useI18n() {
  const ctx = useContext(I18nCtx);
  if (!ctx) throw new Error('useI18n 必须在 I18nProvider 内使用');
  return { lang: ctx.lang, setLang: ctx.setLang, t };
}

/** 顶栏与登录页共用的语言开关（与 ThemeToggle 并排）。 */
export function LanguageToggle() {
  const { lang, setLang } = useI18n();
  return (
    <Select<Lang>
      aria-label={t('layout.lang_toggle')}
      value={lang}
      onChange={setLang}
      style={{ width: 124 }}
      suffixIcon={<GlobalOutlined />}
      options={[
        { value: 'zh-CN', label: t('layout.lang_zh') },
        { value: 'en-US', label: t('layout.lang_en') },
      ]}
    />
  );
}
