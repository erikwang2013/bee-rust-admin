/**
 * 验证码交互的纯逻辑（不碰 DOM/React，单测覆盖）：把用户的操作换算成
 * poster-rust `Answer` 的 serde 表示。**约定都对着插件源码**，改前先看
 * poster-rust 的 `src/captcha/{slider,rotate,click}.rs`：
 *
 * - 坐标一律是**图片像素**（`image` 的原始尺寸），界面按容器宽高缩放时先换算；
 * - slider：`{"Slider": x}` 里 x 是拼图块 PNG 的左上角（jigsaw 下含外扩边），
 *   容差 ±4px；
 * - rotate：`{"Rotate": 度}` 里的度数是**原图被顺时针转过的角度**（插件
 *   `ImageDriver::rotate` 是顺时针，CSS `rotate()` 也是顺时针，方向一致），
 *   而用户是把显示出来的图**转回正**，所以他转的角度与要提交的值互为相反数；
 * - click：`{"Click": [[x, y], …]}` 按点击顺序，与 `extra.texts[].order` 一致，
 *   容差半径 18px。
 */
import type { CaptchaAnswer } from '../../api/types';

/** 显示区里的一次点击/指针位置 → 图片像素坐标。 */
export function toImagePoint(
  clientX: number,
  clientY: number,
  rect: { left: number; top: number; width: number; height: number },
  natural: { width: number; height: number },
): [number, number] {
  // rect 宽高为 0（图片还没加载/被隐藏）时别算出 Infinity：退回 1:1
  const kx = rect.width > 0 ? natural.width / rect.width : 1;
  const ky = rect.height > 0 ? natural.height / rect.height : 1;
  return [
    round2((clientX - rect.left) * kx),
    round2((clientY - rect.top) * ky),
  ];
}

/** 旋转答案：用户把图转了 `cssDeg` 度（CSS 顺时针为正）回正 ⇒ 原图是被转了 -cssDeg 度。 */
export function rotateAnswer(cssDeg: number): CaptchaAnswer {
  const deg = ((-cssDeg % 360) + 360) % 360; // 归一到 [0, 360)：插件按圆周差比对，等价角度都认
  return { Rotate: round2(deg) };
}

/** 滑块答案：x 是拼图块左边缘（图片像素）。 */
export function sliderAnswer(x: number): CaptchaAnswer {
  return { Slider: round2(x) };
}

/** 点击答案：按点击顺序。数量不足时算未完成（返回 null，登录页据此拦提交）。 */
export function clickAnswer(points: [number, number][], expected: number): CaptchaAnswer | null {
  if (expected <= 0 || points.length !== expected) return null;
  return { Click: points };
}

/** 两位小数够了（插件容差是像素/角度级），避免把 IEEE 尾巴塞进请求体。 */
function round2(n: number): number {
  return Math.round(n * 100) / 100;
}
