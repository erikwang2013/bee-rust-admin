import { useEffect, useState } from 'react';
import { dictApi, type DictOption } from '../api/dict';

/**
 * 模块级缓存：字典是「不改代码就能改的配置」，一次会话里不会变，
 * 每个页面各自拉一次纯属浪费。缓存的是 Promise 而不是结果 —— 同一 tick 里
 * 挂载多个下拉时它们共用一次请求。
 */
export const dictCache = new Map<string, Promise<DictOption[]>>();

export function loadDict(code: string): Promise<DictOption[]> {
  const hit = dictCache.get(code);
  if (hit) return hit;
  const p = dictApi.items(code).catch((e: unknown) => {
    // 失败不留缓存：否则一次网络抖动会让这个 code 到刷新页面为止永远空着
    dictCache.delete(code);
    throw e;
  });
  dictCache.set(code, p);
  return p;
}

/** 下拉数据源。错误已由 axios 拦截器统一提示，这里只保证不把 rejection 漏出去。 */
export function useDict(code: string): { options: DictOption[]; loading: boolean } {
  const [options, setOptions] = useState<DictOption[]>([]);
  const [loading, setLoading] = useState(!!code);

  useEffect(() => {
    if (!code) {
      setOptions([]);
      setLoading(false);
      return;
    }
    let alive = true;
    setLoading(true);
    loadDict(code).then(
      (opts) => { if (alive) { setOptions(opts); setLoading(false); } },
      () => { if (alive) { setOptions([]); setLoading(false); } },
    );
    return () => { alive = false; };
  }, [code]);

  return { options, loading };
}
