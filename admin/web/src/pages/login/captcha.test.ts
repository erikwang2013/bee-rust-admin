import { describe, expect, it } from 'vitest';
import {
  MAX_TRAIL_POINTS,
  clickAnswer,
  rotateAnswer,
  sliderAnswer,
  sliderAnswerWithTrail,
  toImagePoint,
  toTrail,
  type RawTrailSample,
} from './captcha';

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

describe('toTrail', () => {
  const rect = { left: 100, top: 50, width: 300, height: 200 };
  const natural = { width: 300, height: 200 };

  it('客户端坐标换算成图片像素，t 是相对起点的毫秒（两位小数）', () => {
    const raw: RawTrailSample[] = [[130, 80, 1000], [160, 90, 1100.456]];
    expect(toTrail(raw, rect, natural)).toEqual([[30, 30, 0], [60, 40, 100.46]]);
  });

  it('单点（按下就抬起）只产出零点；空采样不产出 NaN', () => {
    // t 相对起点 ⇒ 首点恒为 0，duration 也就是末点的 t，不会出现负耗时
    expect(toTrail([[130, 80, 999]], rect, natural)).toEqual([[30, 30, 0]]);
    expect(toTrail([], rect, natural)).toEqual([]);
  });

  it('超过上限按步长抽稀，首末点保留、时间单调不减', () => {
    const raw: RawTrailSample[] = Array.from(
      { length: 1000 },
      (_, i) => [100 + i, 50, 1000 + i * 10] as RawTrailSample,
    );
    const trail = toTrail(raw, rect, natural);
    expect(trail.length).toBeLessThanOrEqual(MAX_TRAIL_POINTS + 1); // 抽稀 + 补回的末点
    expect(trail[0]).toEqual([0, 0, 0]);
    // 末点必留：轨迹的弦取首末两点，丢了末点等于把这条弦挪了位
    expect(trail[trail.length - 1]).toEqual([999, 0, 9990]);
    for (let i = 1; i < trail.length; i++) {
      expect(trail[i][2]).toBeGreaterThanOrEqual(trail[i - 1][2]);
    }
  });
});

describe('sliderAnswerWithTrail', () => {
  it('产出插件 SliderWithTrail 的精确 serde 形状（JSON 逐字）', () => {
    const trail: [number, number, number][] = [[12.01, 3, 0], [173.4, 3.2, 1200]];
    expect(JSON.stringify(sliderAnswerWithTrail(173.4567, trail))).toBe(
      '{"SliderWithTrail":{"x":173.46,"trail":[[12.01,3,0],[173.4,3.2,1200]],"duration_ms":1200}}',
    );
  });

  it('duration_ms 是 u64：按首末点时间差取整（小数会被插件 serde 拒收）', () => {
    const trail: [number, number, number][] = [[0, 0, 0], [10, 0, 1234.56]];
    const a = sliderAnswerWithTrail(100, trail) as {
      SliderWithTrail: { duration_ms: number };
    };
    expect(a.SliderWithTrail.duration_ms).toBe(1235);
    expect(JSON.stringify(a)).toContain('"duration_ms":1235');
  });

  it('toTrail → 构建器：整条请求体的 JSON（后端按这个形状反序列化）', () => {
    const rect = { left: 100, top: 50, width: 300, height: 200 };
    const natural = { width: 300, height: 200 };
    // 「按下停一下再拖」（先犹豫 300ms，再抖一下冲过去）：线性度约 0.77，明显过人那一关
    const raw: RawTrailSample[] = [
      [100, 50, 500],
      [100, 50, 800],
      [104, 50, 950],
      [109, 52, 1100],
      [150, 50, 1500],
    ];
    // 这串字面量在 admin/src/api/auth.rs 的 captcha_trajectory_gate 里也出现（那边反序列化它）
    expect(JSON.stringify(sliderAnswerWithTrail(100, toTrail(raw, rect, natural)))).toBe(
      '{"SliderWithTrail":{"x":100,"trail":[[0,0,0],[0,0,300],[4,0,450],[9,2,600],[50,0,1000]],"duration_ms":1000}}',
    );
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
