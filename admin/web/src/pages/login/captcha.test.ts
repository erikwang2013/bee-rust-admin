import { describe, expect, it } from 'vitest';
import { clickAnswer, rotateAnswer, sliderAnswer, toImagePoint } from './captcha';

describe('toImagePoint', () => {
  const rect = { left: 100, top: 50, width: 300, height: 200 };
  const natural = { width: 300, height: 200 };

  it('1:1 显示时直接减掉偏移', () => {
    expect(toImagePoint(130, 80, rect, natural)).toEqual([30, 30]);
    // 左上角（含 0）与右下角都不越界
    expect(toImagePoint(100, 50, rect, natural)).toEqual([0, 0]);
    expect(toImagePoint(400, 250, rect, natural)).toEqual([300, 200]);
  });

  it('界面缩放时换算回图片像素（图片按 max-width 缩过）', () => {
    // 显示成一半大：同一处点击对应的图片坐标翻倍
    const half = { left: 0, top: 0, width: 150, height: 100 };
    expect(toImagePoint(30, 40, half, natural)).toEqual([60, 80]);
  });

  it('宽高为 0（未加载）时不产出 Infinity/NaN', () => {
    const zero = { left: 0, top: 0, width: 0, height: 0 };
    const [x, y] = toImagePoint(30, 40, zero, natural);
    expect(Number.isFinite(x)).toBe(true);
    expect(Number.isFinite(y)).toBe(true);
  });
});

describe('rotateAnswer', () => {
  it('提交的是原图被转过的角度：与用户转的方向相反', () => {
    // 用户顺时针转 30° 把图转正 ⇒ 原图是被逆时针转了 30°（360-30）
    expect(rotateAnswer(30)).toEqual({ Rotate: 330 });
    // 用户逆时针转 37.2° ⇒ 原图顺时针 37.2°
    expect(rotateAnswer(-37.2)).toEqual({ Rotate: 37.2 });
  });

  it('归一到 [0, 360)，避免送负数（插件按圆周差比对，等价角度都认）', () => {
    expect(rotateAnswer(0)).toEqual({ Rotate: 0 });
    expect(rotateAnswer(360)).toEqual({ Rotate: 0 });
    expect(rotateAnswer(-720.5)).toEqual({ Rotate: 0.5 });
    for (const deg of [-1000, -179.9, 0.1, 179.9, 1000]) {
      const { Rotate } = rotateAnswer(deg) as { Rotate: number };
      expect(Rotate).toBeGreaterThanOrEqual(0);
      expect(Rotate).toBeLessThan(360);
    }
  });
});

describe('sliderAnswer', () => {
  it('保留两位小数', () => {
    expect(sliderAnswer(173)).toEqual({ Slider: 173 });
    expect(sliderAnswer(173.4567)).toEqual({ Slider: 173.46 });
  });
});

describe('clickAnswer', () => {
  it('数量不足算未完成（登录页要拦住提交）', () => {
    expect(clickAnswer([], 3)).toBeNull();
    expect(clickAnswer([[1, 2]], 3)).toBeNull();
    // 数量不符（多了）也不提交：插件按数量严格比对，早交只会白费一次尝试
    expect(clickAnswer([[1, 2], [3, 4], [5, 6], [7, 8]], 3)).toBeNull();
  });

  it('数量对上就按点击顺序原样交出', () => {
    const pts: [number, number][] = [[50, 60], [120, 90], [200, 30]];
    expect(clickAnswer(pts, 3)).toEqual({ Click: pts });
  });
});
