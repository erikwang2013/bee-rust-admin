import { useRef, useState } from 'react';
import { Button, Space, Typography, theme } from 'antd';
import { ReloadOutlined, UndoOutlined } from '@ant-design/icons';
import type { CaptchaAnswer, CaptchaData } from '../../api/types';
import { useI18n } from '../../i18n';
import { clickAnswer, rotateAnswer, sliderAnswer, toImagePoint } from './captcha';

interface Props {
  data: CaptchaData;
  /** 交互完成时回调（未完成传 null），登录页把它拼进登录体；
   *  父组件换验证码时用 `key={data.key}` 重挂本组件，内部状态随之清零。 */
  onAnswer: (a: CaptchaAnswer | null) => void;
  /** 换一张 */
  onRefresh: () => void;
}

/** 归一化到 (-180, 180]，避免拖几圈后角度无限增大。 */
function wrapDeg(deg: number): number {
  return (((deg + 180) % 360) + 360) % 360 - 180;
}

/**
 * 图形验证码交互区（poster-rust 的三种类型）。
 *
 * 坐标口径见 `./captcha.ts` 的模块注释：图片按**原始像素**交互，界面缩放
 * （窄屏 max-width 生效时）靠 `scale` 换算回去。
 */
export default function CaptchaBox({ data, onAnswer, onRefresh }: Props) {
  const { t } = useI18n();
  const { token } = theme.useToken();
  const imgRef = useRef<HTMLImageElement>(null);
  const [scale, setScale] = useState(1);
  /** 最新值放 ref：指针抬起要用的值不能等 React 的下一次渲染 */
  const xRef = useRef(0);
  const degRef = useRef(0);
  const drag = useRef<{ px: number; x: number } | null>(null);
  const rot = useRef<{ start: number; deg: number } | null>(null);

  const [x, setX] = useState(0);
  const [deg, setDeg] = useState(0);
  const [points, setPoints] = useState<[number, number][]>([]);

  const targets = data.extra.texts ?? [];
  const maxX = (imgRef.current?.naturalWidth ?? 300) - (data.extra.puzzle_w ?? 50);

  /** 图片的显示尺寸 ÷ 原始尺寸：原尺寸显示时为 1，窄屏被 max-width 压过才小于 1。 */
  const onLoad = (el: HTMLImageElement) => setScale(el.clientWidth / el.naturalWidth || 1);

  // ── 滑块：拖动拼图块 ────────────────────────────────
  const onPieceDown = (e: React.PointerEvent<HTMLImageElement>) => {
    drag.current = { px: e.clientX, x: xRef.current };
    e.currentTarget.setPointerCapture(e.pointerId);
  };
  const onPieceMove = (e: React.PointerEvent<HTMLImageElement>) => {
    const d = drag.current;
    if (!d) return;
    const next = Math.min(Math.max(d.x + (e.clientX - d.px) / scale, 0), Math.max(maxX, 0));
    xRef.current = next;
    setX(next);
  };
  const onPieceUp = () => {
    if (!drag.current) return;
    drag.current = null;
    onAnswer(sliderAnswer(xRef.current));
  };

  // ── 旋转：拖动图片 ──────────────────────────────────
  /** 指针相对图片中心的角度（屏幕坐标 y 向下，顺时针增大 —— 与 CSS rotate 同向）。 */
  const angleAt = (e: React.PointerEvent<HTMLImageElement>) => {
    const r = e.currentTarget.getBoundingClientRect();
    return (Math.atan2(e.clientY - (r.top + r.height / 2), e.clientX - (r.left + r.width / 2)) * 180) / Math.PI;
  };
  const onRotDown = (e: React.PointerEvent<HTMLImageElement>) => {
    rot.current = { start: angleAt(e), deg: degRef.current };
    e.currentTarget.setPointerCapture(e.pointerId);
  };
  const onRotMove = (e: React.PointerEvent<HTMLImageElement>) => {
    const r = rot.current;
    if (!r) return;
    const next = wrapDeg(r.deg + (angleAt(e) - r.start));
    degRef.current = next;
    setDeg(next);
  };
  const onRotUp = () => {
    if (!rot.current) return;
    rot.current = null;
    onAnswer(rotateAnswer(degRef.current));
  };

  // ── 点击：按提示顺序点图 ────────────────────────────
  const onImageDown = (e: React.PointerEvent<HTMLImageElement>) => {
    if (points.length >= targets.length) return; // 点够了就停，多出来的点击会白费一次尝试
    const im = e.currentTarget;
    const next: [number, number][] = [
      ...points,
      toImagePoint(e.clientX, e.clientY, im.getBoundingClientRect(), {
        width: im.naturalWidth,
        height: im.naturalHeight,
      }),
    ];
    setPoints(next);
    onAnswer(clickAnswer(next, targets.length));
  };

  const box: React.CSSProperties = {
    position: 'relative',
    display: 'inline-block',
    lineHeight: 0,
    border: `1px solid ${token.colorBorderSecondary}`,
    borderRadius: token.borderRadius,
    overflow: 'hidden',
    userSelect: 'none',
    touchAction: 'none', // 触屏拖动时别把页面一起滚了
  };
  // click / slider 的图是背景，指针事件挂它上面；rotate 的图要跟着转，单独一份样式
  const baseImg: React.CSSProperties = { display: 'block', maxWidth: '100%' };

  const hint =
    data.type === 'click'
      ? t('login.captcha_click_hint')
      : data.type === 'rotate'
        ? t('login.captcha_rotate_hint')
        : t('login.captcha_slider_hint');

  return (
    <Space direction="vertical" size={6} style={{ width: '100%' }}>
      <Typography.Text type="secondary" style={{ fontSize: 12 }}>
        {hint}
      </Typography.Text>

      {data.type === 'click' && (
        <Space size={4} wrap style={{ rowGap: 4 }}>
          {targets.map((tg) => (
            <span
              key={tg.order}
              style={{
                padding: '0 6px',
                border: `1px solid ${token.colorBorderSecondary}`,
                borderRadius: token.borderRadiusSM,
                fontSize: 13,
                lineHeight: '20px',
              }}
            >
              {tg.order}.{' '}
              {tg.thumb ? <img src={tg.thumb} alt="" height={16} style={{ verticalAlign: '-3px' }} /> : tg.text}
            </span>
          ))}
        </Space>
      )}

      <div style={box}>
        {data.type === 'rotate' ? (
          <img
            ref={imgRef}
            src={data.image}
            alt=""
            draggable={false}
            onLoad={(e) => onLoad(e.currentTarget)}
            onPointerDown={onRotDown}
            onPointerMove={onRotMove}
            onPointerUp={onRotUp}
            onPointerCancel={onRotUp}
            style={{ ...baseImg, transform: `rotate(${deg}deg)`, cursor: 'grab' }}
          />
        ) : (
          <img
            ref={imgRef}
            src={data.image}
            alt=""
            draggable={false}
            onLoad={(e) => onLoad(e.currentTarget)}
            onPointerDown={data.type === 'click' ? onImageDown : undefined}
            style={{ ...baseImg, cursor: data.type === 'click' ? 'crosshair' : 'default' }}
          />
        )}

        {data.type === 'slider' && data.extra.puzzle && (
          <img
            src={data.extra.puzzle}
            alt=""
            draggable={false}
            onPointerDown={onPieceDown}
            onPointerMove={onPieceMove}
            onPointerUp={onPieceUp}
            onPointerCancel={onPieceUp}
            style={{
              position: 'absolute',
              left: x * scale,
              top: '50%',
              transform: 'translateY(-50%)',
              cursor: 'ew-resize',
            }}
          />
        )}

        {data.type === 'click' &&
          points.map(([px, py], i) => (
            <span
              key={`${px}-${py}-${i}`}
              style={{
                position: 'absolute',
                left: px * scale - 9,
                top: py * scale - 9,
                width: 18,
                height: 18,
                borderRadius: '50%',
                background: token.colorPrimary,
                color: token.colorTextLightSolid,
                fontSize: 12,
                lineHeight: '18px',
                textAlign: 'center',
              }}
            >
              {i + 1}
            </span>
          ))}
      </div>

      <Space size={8}>
        <Button size="small" icon={<ReloadOutlined />} onClick={onRefresh}>
          {t('login.captcha_refresh')}
        </Button>
        {data.type === 'click' && points.length > 0 && (
          <Button
            size="small"
            icon={<UndoOutlined />}
            onClick={() => {
              const next = points.slice(0, -1);
              setPoints(next);
              onAnswer(clickAnswer(next, targets.length));
            }}
          >
            {t('login.captcha_undo')}
          </Button>
        )}
      </Space>
    </Space>
  );
}
