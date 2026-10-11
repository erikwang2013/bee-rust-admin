/**
 * 密码复杂度规则，与后端 `admin/src/api/admin.rs::validate_password` 同口径：
 *
 * - 长度 ≥ 8，按**字符数**算（`[...pw].length`，不是 `pw.length`：后者数的是
 *   UTF-16 码元，emoji 会一个顶俩）；
 * - 四类里至少两类：小写字母 / 大写字母 / 数字 / 其它。类别按 **ASCII** 划，
 *   「其它」= 非 ASCII 字母数字（符号、汉字、emoji 都算）。
 *
 * 只用在**新设 / 新改**密码的表单上；登录页不校验（存量密码不受影响）。
 * 规则只有这一份，三处表单都调它；后端那边也只有一份，两边必须一起改。
 */
export function isStrongPassword(pw: string): boolean {
  if ([...pw].length < 8) return false;
  const classes = [/[a-z]/, /[A-Z]/, /[0-9]/, /[^a-zA-Z0-9]/].filter((re) => re.test(pw)).length;
  return classes >= 2;
}
