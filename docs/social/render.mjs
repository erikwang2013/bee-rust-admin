// BRA 社交预览图（GitHub Social Preview，1280×640）渲染器。
//
// 用法： node docs/social/render.mjs          # 出 preview.svg
//        rsvg-convert -w 1280 docs/social/preview.svg -o docs/social/preview.png
//
// GitHub 的推荐尺寸是 1280×640（最小 640×320），**只收 PNG/JPG/GIF，不吃 SVG**，
// 所以 SVG 是源文件、PNG 才是要上传的那份。设置入口：仓库 Settings → Social preview。
//
// 设计要点：
//   - 深色底：与后台暗色主题同一族色（slate 深蓝），暖琥珀的吉祥物在深底上才跳得出来
//   - 背景铺一层极淡的六边形蜂巢网格：呼应主题，又不会在缩略图里糊成一片
//   - 吉祥物用 keeper-onduty（「值守」态：门岗正常、权限表与菜单树一致）——
//     社交图要的是「这个项目在干嘛」，不是某个告警态
//   - 信息分三层：名字 → 一句话 → 能力标签；缩略图（Slack/微信）只看得到第一层
//   - 字体用思源黑体，中英文一套字重解决，避免中文回退成宋体

import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = join(HERE, '..', '..');

const W = 1280;
const H = 640;

// ── 调色：与后台暗色主题、吉祥物同源 ─────────────────────────
const BG_TOP = '#0A1020';
const BG_BOT = '#152036';
const INK = '#3A2A1A'; // 吉祥物描边色（只用于说明，不直接画）
const AMBER = '#FFC12B';
const AMBER_LIGHT = '#FFD75E';
const WHITE = '#F8FAFC';
const BODY = '#CBD5E1';
const MUTED = '#8A9AB4';
const FAINT = '#5B6B85';
const TEAL = '#7FC8DE';

const FONT = 'Source Han Sans SC';

const esc = (s) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');

// 粗略估宽：CJK/全角按 1em，其余按 0.56em。只用于算标签底色，够用。
const textWidth = (s, size) =>
  [...s].reduce((w, ch) => w + (/[⺀-鿿＀-｠　-〿]/.test(ch) ? 1 : 0.56), 0) * size;

// ── 背景：斜向渐变 + 淡淡的蜂巢网格 ─────────────────────────
function honeycomb() {
  const R = 46; // 外接圆半径
  const dx = Math.sqrt(3) * R;
  const dy = 1.5 * R;
  const rows = Math.ceil(H / dy) + 2;
  const cols = Math.ceil(W / dx) + 2;
  const pts = [];
  for (let r = -1; r < rows; r++) {
    for (let c = -1; c < cols; c++) {
      const cx = c * dx + (r % 2 ? dx / 2 : 0);
      const cy = r * dy;
      const corners = Array.from({ length: 6 }, (_, i) => {
        const a = (Math.PI / 180) * (60 * i - 90);
        return `${(cx + R * Math.cos(a)).toFixed(1)},${(cy + R * Math.sin(a)).toFixed(1)}`;
      }).join(' ');
      pts.push(`<polygon points="${corners}"/>`);
    }
  }
  return `<g fill="none" stroke="#3E5A80" stroke-width="1.2" opacity="0.16">${pts.join('')}</g>`;
}

// 尖顶正六边形的六个顶点（与 keeper-app 图标同一造型）
function hexPoints(cx, cy, r) {
  return Array.from({ length: 6 }, (_, i) => {
    const a = (Math.PI / 180) * (60 * i - 90);
    return `${(cx + r * Math.cos(a)).toFixed(1)},${(cy + r * Math.sin(a)).toFixed(1)}`;
  });
}

// ── 能力标签 ────────────────────────────────────────────────
function chips(items, x, y, opts = {}) {
  const { size = 19, padX = 18, gap = 12, rowGap = 14, maxWidth = 660 } = opts;
  const out = [];
  let cx = x;
  let cy = y;
  for (const label of items) {
    const w = textWidth(label, size) + padX * 2;
    if (cx + w > x + maxWidth) {
      cx = x;
      cy += size + 24 + rowGap;
    }
    out.push(
      `<rect x="${cx.toFixed(1)}" y="${cy.toFixed(1)}" width="${w.toFixed(1)}" height="${size + 24}" rx="${(size + 24) / 2}"` +
        ` fill="#FFFFFF" fill-opacity="0.06" stroke="#7FC8DE" stroke-opacity="0.35"/>`,
      `<text x="${(cx + w / 2).toFixed(1)}" y="${cy + size + 6.5}" font-family="${FONT}" font-size="${size}"` +
        ` fill="${BODY}" text-anchor="middle">${esc(label)}</text>`,
    );
    cx += w + gap;
  }
  return out.join('\n    ');
}

// ── 吉祥物：把 SVG 内联进来（外用 <g> 定位缩放） ──────────────
function mascot() {
  const raw = readFileSync(join(ROOT, 'docs', 'pet', 'keeper-onduty.svg'), 'utf8');
  const inner = raw
    .replace(/^[\s\S]*?<svg[^>]*>/, '')
    .replace(/<\/svg>\s*$/, '')
    // 呼吸动画在静态图里没有意义，去掉（留着也不影响渲染，只是白占体积）
    // 底部的落地投影保留：它是深棕 12% 的椭圆，垫在浅色六边形上正好有落地感
    .replace(/<animateTransform[\s\S]*?\/>/g, '');

  // 原始画布 512×512。置于六边形底板内：让吉祥物的视觉中心落在底板中心 (300,322)。
  // 系数按「画布中心 (255,225) → 底板中心」推出来，改底板半径时同步调。
  const S = 0.88;
  const x0 = 300 - 255 * S;
  const y0 = 322 - 225 * S;
  return `<g transform="translate(${x0.toFixed(1)},${y0.toFixed(1)}) scale(${S})">${inner}</g>`;
}

// ── 组装 ────────────────────────────────────────────────────
const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${W}" height="${H}" viewBox="0 0 ${W} ${H}" role="img"
     aria-label="BRA · Bee Rust Admin —— 基于 bee-rust 框架的 RBAC 管理后台">
  <title>BRA · Bee Rust Admin</title>
  <desc>基于 bee-rust 框架的 RBAC 管理后台：Rust 服务端 + React 前端。菜单与按钮级权限、部门数据权限、操作留痕、中英双语。</desc>
  <defs>
    <linearGradient id="bg" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0%" stop-color="${BG_TOP}"/>
      <stop offset="100%" stop-color="${BG_BOT}"/>
    </linearGradient>
    <linearGradient id="title" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0%" stop-color="${AMBER_LIGHT}"/>
      <stop offset="100%" stop-color="#F0A81E"/>
    </linearGradient>
    <radialGradient id="glow" cx="50%" cy="50%" r="50%">
      <stop offset="0%" stop-color="#FFE9AE" stop-opacity="0.30"/>
      <stop offset="45%" stop-color="${AMBER}" stop-opacity="0.13"/>
      <stop offset="100%" stop-color="${AMBER}" stop-opacity="0"/>
    </radialGradient>
    <linearGradient id="plate" x1="0.2" y1="0" x2="0.8" y2="1">
      <stop offset="0%" stop-color="#FFFBF0"/>
      <stop offset="100%" stop-color="#FFE7AE"/>
    </linearGradient>
  </defs>

  <rect width="${W}" height="${H}" fill="url(#bg)"/>
  ${honeycomb()}
  <circle cx="300" cy="320" r="330" fill="url(#glow)"/>

  <!-- 吉祥物垫在浅色六边形上：它的描边与下半身是深棕色的，直接压在深蓝底上会糊成一团。
       六边形既解决对比度，又和 keeper-app 图标、背景蜂巢是同一套造型语言。 -->
  <polygon points="${hexPoints(300, 320, 252).join(' ')}"
           fill="url(#plate)" stroke="#F0A81E" stroke-opacity="0.55" stroke-width="2"/>

  ${mascot()}

  <g>
    <text x="596" y="150" font-family="${FONT}" font-size="21" font-weight="bold"
          fill="${TEAL}" letter-spacing="5">BEE RUST ADMIN</text>

    <text x="592" y="266" font-family="${FONT}" font-size="112" font-weight="bold" fill="url(#title)">BRA</text>

    <text x="596" y="330" font-family="${FONT}" font-size="35" font-weight="bold" fill="${WHITE}">基于 bee-rust 框架的管理后台</text>

    <text x="596" y="376" font-family="${FONT}" font-size="21" fill="${MUTED}">Rust 服务端 + React 前端 · 启动即建表，开箱可用</text>

    ${chips(['菜单 + 按钮级权限', '部门数据权限', '中英双语'], 596, 424)}
    ${chips(['字典管理', '定时任务', '操作留痕'], 596, 490)}

    <line x1="596" y1="576" x2="1184" y2="576" stroke="#FFFFFF" stroke-opacity="0.10"/>

    <text x="596" y="608" font-family="${FONT}" font-size="18" fill="${FAINT}">github.com/erikwang2013/bee-rust-admin</text>
    <text x="1184" y="608" font-family="${FONT}" font-size="18" fill="${FAINT}" text-anchor="end">© erik.xyz</text>
  </g>
</svg>
`;

writeFileSync(join(HERE, 'preview.svg'), svg);
console.log(`wrote docs/social/preview.svg  (${W}×${H}, ${(svg.length / 1024).toFixed(1)} kB)`);
