/**
 * 请求代际：只认最后一次发起的请求。
 *
 * `begin()` 给新请求发个号，响应回来用 `isCurrent(token)` 判断是否已被更新的请求顶掉；
 * `invalidate()` 不发请求、只作废在途的（关弹窗 / 切新建 / 换篇时用），慢响应回来直接丢。
 */
export function latestGuard() {
  let seq = 0;
  return {
    begin: () => ++seq,
    invalidate: () => { seq += 1; },
    isCurrent: (token: number) => token === seq,
  };
}
