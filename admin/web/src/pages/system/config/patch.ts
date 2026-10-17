import type { ConfigUpdateForm } from '../../../api/config';
import type { SysConfig } from '../../../api/types';

/**
 * 一项配置相对**服务端当前值**的部分更新：只带真正变了的字段，没变 = null。
 * 「值」与「公开开关」分开判断：PUT 是部分更新，改值不该顺手重写公开状态，反之亦然。
 * 开关在表单里是 boolean，后端要 0/1 整数，归一化只在这一处做。
 */
export function configPatch(
  item: Pick<SysConfig, 'value' | 'is_public'>,
  nextValue: string,
  nextPublic: boolean,
): ConfigUpdateForm | null {
  const patch: ConfigUpdateForm = {};
  if (nextValue !== item.value) patch.value = nextValue;
  if (nextPublic !== (item.is_public === 1)) patch.is_public = nextPublic ? 1 : 0;
  return Object.keys(patch).length === 0 ? null : patch;
}
