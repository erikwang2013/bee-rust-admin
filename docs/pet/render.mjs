// 阿守（Keeper）五态渲染器 —— 一份模板 + 每种心情的差异片段。
//
// 用法： node docs/pet/render.mjs
// 输出： docs/pet/keeper-{onduty,verifying,superuser,dozing,alarmed}.svg
//
// 设计要点（第二版重画）：
//   - Q 版比例：大头 + 圆胖身体，头比身体更抢眼
//   - 粗描边贴纸风 + 大眼睛带高光 + 腮红，小尺寸下也认得出
//   - 粗短的圆润四肢，不用细线条（细腿在剪影上像昆虫标本）
//   - 只留一个招牌道具（钥匙），其余靠表情与姿态区分
//   - 元素做减法：去掉巢门平台，蜂巢意象收进胸章

const W = 512;
const H = 512;

const INK = '#3A2A1A'; // 统一暖炭色描边
const LINE = 5;

// ── 翅膀：默认收在背后；超管展开、打盹下垂、警戒高举 ──────────
const wings = (mode) => {
  const shapes = {
    rest: [
      'M186,258 C130,212 108,166 116,128 C158,142 190,180 202,224 Z',
      'M326,258 C382,212 404,166 396,128 C354,142 322,180 310,224 Z',
    ],
    spread: [
      'M176,244 C104,190 66,126 62,74 C124,84 176,128 198,186 Z',
      'M336,244 C408,190 446,126 450,74 C388,84 336,128 314,186 Z',
    ],
    droop: [
      'M192,268 C148,264 116,252 96,236 C130,208 176,214 204,240 Z',
      'M320,268 C364,264 396,252 416,236 C382,208 336,214 308,240 Z',
    ],
    alert: [
      'M182,240 C112,168 78,104 76,52 C142,70 192,120 206,180 Z',
      'M330,240 C400,168 434,104 436,52 C370,70 320,120 306,180 Z',
    ],
  }[mode];
  return `<g>
    ${shapes
      .map((d) => `<path d="${d}" fill="#E4F2FB" stroke="#8FBED8" stroke-width="${LINE - 1}"/>`)
      .join('\n    ')}
  </g>`;
};

// ── 大眼睛（带上高光，可爱度的关键）──────────────────────────
const eyeOpen = (cx, cy, r = 1) => `
    <ellipse cx="${cx}" cy="${cy}" rx="${18 * r}" ry="${22 * r}" fill="${INK}"/>
    <circle cx="${cx - 6}" cy="${cy - 8}" r="${7 * r}" fill="#FFFFFF"/>
    <circle cx="${cx + 6}" cy="${cy + 9}" r="${3.4 * r}" fill="#FFFFFF" opacity="0.85"/>`;

// ── 表情：五种心情的差异集中在这里 ───────────────────────────
const face = (mood) => {
  if (mood === 'superuser') {
    return `
    <g stroke="${INK}" stroke-width="6" fill="none" stroke-linecap="round">
      <path d="M206,170 Q224,148 242,170"/>
      <path d="M270,170 Q288,148 306,170"/>
    </g>
    <path d="M234,196 Q256,220 278,196" fill="none" stroke="${INK}" stroke-width="5.5" stroke-linecap="round"/>
    <path d="M240,199 Q256,214 272,199" fill="#B4552A" opacity="0.35"/>`;
  }
  if (mood === 'dozing') {
    return `
    <g stroke="${INK}" stroke-width="6" fill="none" stroke-linecap="round">
      <path d="M205,172 Q224,184 243,172"/>
      <path d="M269,172 Q288,184 307,172"/>
    </g>
    <ellipse cx="256" cy="202" rx="9" ry="12" fill="#B4552A" opacity="0.75"/>`;
  }
  if (mood === 'alarmed') {
    return `
    ${eyeOpen(224, 172, 0.94)}
    ${eyeOpen(288, 172, 0.94)}
    <g stroke="${INK}" stroke-width="6.5" fill="none" stroke-linecap="round">
      <path d="M200,140 L240,154"/>
      <path d="M312,140 L272,154"/>
    </g>
    <path d="M234,204 L278,204" stroke="${INK}" stroke-width="5.5" stroke-linecap="round"/>
    <g fill="#FFFFFF" opacity="0.95">
      <rect x="240" y="197" width="9" height="14" rx="2"/>
      <rect x="263" y="197" width="9" height="14" rx="2"/>
    </g>`;
  }
  if (mood === 'verifying') {
    return `
    ${eyeOpen(224, 172)}
    ${eyeOpen(292, 172, 1.18)}
    <path d="M198,136 L234,146" stroke="${INK}" stroke-width="6" fill="none" stroke-linecap="round"/>
    <ellipse cx="256" cy="204" rx="10" ry="11" fill="#B4552A" opacity="0.7"/>`;
  }
  // 值守（默认）：柔和微笑
  return `
    ${eyeOpen(224, 170)}
    ${eyeOpen(288, 170)}
    <path d="M240,198 Q256,212 272,198" fill="none" stroke="${INK}" stroke-width="5.5" stroke-linecap="round"/>`;
};

// ── 胸章：对勾（通过）/ 金星（超管）/ 感叹号（告警）───────────
const badge = (kind) => {
  const hex = '<polygon points="256,264 277,276 277,300 256,312 235,300 235,276"';
  const fill = kind === 'star' ? '#E8B33A' : kind === 'alert' ? '#D64545' : '#FFFFFF';
  const stroke = kind === 'star' ? '#8A5A0B' : kind === 'alert' ? '#8E2020' : '#2E8FA8';
  const mark =
    kind === 'star'
      ? `<path d="M256,272 L261,285 L275,286 L264,295 L268,309 L256,301 L244,309 L248,295 L237,286 L251,285 Z" fill="#8A5A0B"/>`
      : kind === 'alert'
        ? `<path d="M256,274 L256,294" stroke="#FFFFFF" stroke-width="6" stroke-linecap="round"/>
           <circle cx="256" cy="304" r="4" fill="#FFFFFF"/>`
        : `<path d="M245,289 L253,298 L268,279" fill="none" stroke="#2E8FA8" stroke-width="5"
                 stroke-linecap="round" stroke-linejoin="round"/>`;
  return `${hex} fill="${fill}" stroke="${stroke}" stroke-width="3.5"/>\n    ${mark}`;
};

// ── 招牌道具：一把钥匙（挂在右手）──────────────────────────
const key = (x, y, dropped = false) => `
    <g ${dropped ? 'opacity="0.85"' : ''}>
      <circle cx="${x}" cy="${y}" r="14" fill="none" stroke="${INK}" stroke-width="8"/>
      <path d="M${x},${y + 14} L${x},${y + 60}" stroke="${INK}" stroke-width="10" stroke-linecap="round"/>
      <path d="M${x},${y + 40} L${x - 18},${y + 40}" stroke="${INK}" stroke-width="8" stroke-linecap="round"/>
      <path d="M${x},${y + 54} L${x - 13},${y + 54}" stroke="${INK}" stroke-width="8" stroke-linecap="round"/>
    </g>`;

// ── 附加道具 / 氛围 ─────────────────────────────────────────
const extras = (mood) => {
  if (mood === 'verifying') {
    // 举放大镜 + 问号
    return `
  <path d="M150,306 C126,314 118,328 122,344" stroke="${INK}" stroke-width="13" fill="none" stroke-linecap="round"/>
  <g transform="rotate(24 116 300)">
    <circle cx="116" cy="300" r="30" fill="#E4F2FB" fill-opacity="0.6" stroke="${INK}" stroke-width="6"/>
    <path d="M139,323 L166,352" stroke="${INK}" stroke-width="10" stroke-linecap="round"/>
    <path d="M100,288 Q112,278 128,282" stroke="#FFFFFF" stroke-width="6" fill="none" stroke-linecap="round" opacity="0.9"/>
  </g>
  <text x="336" y="128" font-family="system-ui,'PingFang SC',sans-serif" font-size="52" font-weight="800"
        fill="#2E8FA8">?</text>`;
  }
  if (mood === 'dozing') {
    return `
  <g fill="#2E8FA8" font-family="ui-monospace,Menlo,monospace" font-weight="800">
    <text x="336" y="152" font-size="30">z</text>
    <text x="360" y="122" font-size="23">z</text>
    <text x="380" y="96" font-size="17">z</text>
  </g>
  <path d="M96,104 A44,44 0 0 1 130,70 A38,38 0 0 0 86,90 Z" fill="#F7D774" stroke="#C99A2E" stroke-width="2.5"/>`;
  }
  if (mood === 'alarmed') {
    return `
  <circle cx="256" cy="300" r="190" fill="#E45B5B" opacity="0.10"/>
  <g transform="rotate(-12 112 150)">
    <rect x="76" y="118" width="72" height="88" rx="6" fill="#FFF7ED" stroke="#C03030" stroke-width="3.5"/>
    <text x="112" y="172" text-anchor="middle" font-family="ui-monospace,Menlo,monospace"
          font-size="22" font-weight="700" fill="#C03030">.env</text>
  </g>`;
  }
  if (mood === 'superuser') {
    return `
  <circle cx="256" cy="290" r="190" fill="#F7D774" opacity="0.20"/>
  <path d="M204,92 L216,56 L238,80 L256,46 L274,80 L296,56 L308,92 Z"
        fill="#E8B33A" stroke="${INK}" stroke-width="5" stroke-linejoin="round"/>
  <g fill="#F0C64A">
    <path d="M120,182 l6,14 14,6 -14,6 -6,14 -6,-14 -14,-6 14,-6 Z"/>
    <path d="M396,166 l5,12 12,5 -12,5 -5,12 -5,-12 -12,-5 12,-5 Z"/>
  </g>`;
  }
  return '';
};

// ── 尾针：在岗收成小圆点，警戒时拉长外挑 ─────────────────────
const sting = (raised) =>
  raised
    ? `<path d="M352,392 L416,428 L360,424 Z" fill="#E8890C" stroke="${INK}" stroke-width="4" stroke-linejoin="round"/>`
    : `<path d="M356,404 L382,418" stroke="${INK}" stroke-width="9" stroke-linecap="round"/>`;

const svg = (mood, label) => `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${W} ${H}" width="${W}" height="${H}" role="img" aria-label="阿守（Keeper）· ${label}">
  <title>阿守（Keeper）· ${label}</title>
  <desc>BRA 项目宠物的「${label}」状态。</desc>
  <defs>
    <radialGradient id="bodyGrad" cx="42%" cy="26%" r="82%">
      <stop offset="0%" stop-color="#FFD75E"/><stop offset="62%" stop-color="#FFC12B"/><stop offset="100%" stop-color="#F0A81E"/>
    </radialGradient>
    <radialGradient id="headGrad" cx="40%" cy="26%" r="82%">
      <stop offset="0%" stop-color="#FFE28A"/><stop offset="100%" stop-color="#FFBF2E"/>
    </radialGradient>
    <radialGradient id="glowGrad" cx="50%" cy="50%" r="50%">
      <stop offset="0%" stop-color="#F0C64A" stop-opacity="0.55"/><stop offset="100%" stop-color="#F0C64A" stop-opacity="0"/>
    </radialGradient>
  </defs>

  <ellipse cx="256" cy="462" rx="118" ry="16" fill="${INK}" opacity="0.12"/>
  ${wings(mood === 'superuser' ? 'spread' : mood === 'dozing' ? 'droop' : mood === 'alarmed' ? 'alert' : 'rest')}
  ${extras(mood)}

  <!-- 四肢（先画，压在身体下层）：粗短的圆润手脚 -->
  <g stroke="${INK}" stroke-width="14" stroke-linecap="round" fill="none">
    <path d="M148,308 C122,318 114,334 120,350"/>
    ${mood === 'verifying' ? '' : '<path d="M364,308 C390,318 398,334 392,350"/>'}
  </g>

  <!-- 身体 + 条纹 + 尾针 + 脚 -->
  <g>
    <ellipse cx="256" cy="316" rx="116" ry="118" fill="url(#bodyGrad)" stroke="${INK}" stroke-width="${LINE}"/>
    <path d="M150,330 Q256,366 362,330" fill="none" stroke="${INK}" stroke-width="26" stroke-linecap="round"/>
    <path d="M170,390 Q256,418 342,390" fill="none" stroke="${INK}" stroke-width="24" stroke-linecap="round"/>
    ${sting(mood === 'alarmed')}
    <ellipse cx="216" cy="428" rx="27" ry="17" fill="url(#bodyGrad)" stroke="${INK}" stroke-width="${LINE}"/>
    <ellipse cx="296" cy="428" rx="27" ry="17" fill="url(#bodyGrad)" stroke="${INK}" stroke-width="${LINE}"/>
  </g>

  <!-- 胸章 -->
  <g>${badge(mood === 'superuser' ? 'star' : mood === 'alarmed' ? 'alert' : 'check')}</g>

  <!-- 钥匙（右手提着；打盹时脱手吊着）-->
  ${mood === 'verifying' ? '' : key(392, 356, mood === 'dozing')}

  <!-- 头（最后画，压住身体上沿）-->
  <g transform="${mood === 'dozing' ? 'rotate(8 256 190)' : ''}">
    <path d="M208,104 C190,76 176,62 160,54" stroke="${INK}" stroke-width="6" fill="none" stroke-linecap="round"/>
    <path d="M304,104 C322,76 336,62 352,54" stroke="${INK}" stroke-width="6" fill="none" stroke-linecap="round"/>
    <circle cx="157" cy="52" r="9" fill="${INK}"/>
    <circle cx="355" cy="52" r="9" fill="${INK}"/>
    <circle cx="256" cy="172" r="88" fill="url(#headGrad)" stroke="${INK}" stroke-width="${LINE}"/>
    <ellipse cx="198" cy="200" rx="16" ry="10" fill="#E88B6A" opacity="0.5"/>
    <ellipse cx="314" cy="200" rx="16" ry="10" fill="#E88B6A" opacity="0.5"/>
    ${face(mood)}
    <animateTransform attributeName="transform" type="translate" values="0,0; 0,-4; 0,0"
                      dur="${mood === 'dozing' ? '5.5s' : '2.6s'}" repeatCount="indefinite"/>
  </g>
</svg>
`;

// ── 应用图标：六边形底 + 头（favicon / 应用图标；16px 下只留最大对比度的形）──
// 小尺寸下细节全糊，所以这里刻意简化：粗描边、大眼、无斑纹、无道具。
const appIcon = () => `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256" width="256" height="256" role="img" aria-label="BRA 应用图标：阿守的头像">
  <title>BRA · 阿守</title>
  <defs>
    <radialGradient id="ih" cx="40%" cy="28%" r="80%">
      <stop offset="0%" stop-color="#FFE28A"/><stop offset="100%" stop-color="#FFBF2E"/>
    </radialGradient>
  </defs>
  <polygon points="128,8 236,70 236,186 128,248 20,186 20,70" fill="#FFC12B" stroke="${INK}" stroke-width="12" stroke-linejoin="round"/>
  <circle cx="128" cy="140" r="78" fill="url(#ih)" stroke="${INK}" stroke-width="10"/>
  <g stroke="${INK}" stroke-width="11" fill="none" stroke-linecap="round">
    <path d="M92,76 C76,54 66,44 54,38"/>
    <path d="M164,76 C180,54 190,44 202,38"/>
  </g>
  <circle cx="52" cy="36" r="11" fill="${INK}"/>
  <circle cx="204" cy="36" r="11" fill="${INK}"/>
  <ellipse cx="98" cy="136" rx="19" ry="23" fill="${INK}"/>
  <ellipse cx="158" cy="136" rx="19" ry="23" fill="${INK}"/>
  <circle cx="92" cy="127" r="7.5" fill="#FFFFFF"/>
  <circle cx="152" cy="127" r="7.5" fill="#FFFFFF"/>
  <path d="M110,176 Q128,192 146,176" fill="none" stroke="${INK}" stroke-width="7" stroke-linecap="round"/>
</svg>
`;

// ── 透明底头像：侧栏 Logo / 默认头像用（放在浅色背景上，不带六边形底）──
const head = () => `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256" width="256" height="256" role="img" aria-label="阿守（Keeper）头像">
  <title>阿守 · Keeper</title>
  <defs>
    <radialGradient id="hh" cx="40%" cy="28%" r="80%">
      <stop offset="0%" stop-color="#FFE28A"/><stop offset="100%" stop-color="#FFBF2E"/>
    </radialGradient>
  </defs>
  <g stroke="${INK}" stroke-width="7" fill="none" stroke-linecap="round">
    <path d="M96,84 C80,60 70,50 56,42"/>
    <path d="M160,84 C176,60 186,50 200,42"/>
  </g>
  <circle cx="54" cy="40" r="8" fill="${INK}"/>
  <circle cx="202" cy="40" r="8" fill="${INK}"/>
  <circle cx="128" cy="152" r="92" fill="url(#hh)" stroke="${INK}" stroke-width="6"/>
  <ellipse cx="68" cy="186" rx="16" ry="10" fill="#E88B6A" opacity="0.5"/>
  <ellipse cx="188" cy="186" rx="16" ry="10" fill="#E88B6A" opacity="0.5"/>
  <ellipse cx="100" cy="150" rx="18" ry="22" fill="${INK}"/>
  <ellipse cx="156" cy="150" rx="18" ry="22" fill="${INK}"/>
  <circle cx="94" cy="142" r="7" fill="#FFFFFF"/>
  <circle cx="150" cy="142" r="7" fill="#FFFFFF"/>
  <path d="M112,188 Q128,202 144,188" fill="none" stroke="${INK}" stroke-width="5.5" stroke-linecap="round"/>
</svg>
`;

const moods = [
  ['onduty', '值守'],
  ['verifying', '查证'],
  ['superuser', '超管'],
  ['dozing', '打盹'],
  ['alarmed', '竖针'],
];

import { writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
const here = dirname(fileURLToPath(import.meta.url));
for (const [slug, label] of moods) {
  writeFileSync(join(here, `keeper-${slug}.svg`), svg(slug, label), 'utf8');
  console.log('written', `keeper-${slug}.svg`, `(${label})`);
}
writeFileSync(join(here, 'keeper-head.svg'), head(), 'utf8');
console.log('written keeper-head.svg (透明底头像)');
writeFileSync(join(here, 'keeper-app.svg'), appIcon(), 'utf8');
console.log('written keeper-app.svg (应用图标)');
