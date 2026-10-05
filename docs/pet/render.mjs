// 阿守（Keeper）五态渲染器 —— 一份模板 + 每种心情的差异片段。
//
// 用法： node docs/pet/render.mjs
// 输出： docs/pet/keeper-onduty.svg / -verifying / -superuser / -dozing / -alarmed
//
// 设计约束：五种状态共用同一副身体（改一条腿只需改这里一处），
// 只让「脸 / 翅膀姿态 / 尾针 / 胸章 / 附加道具」随心情变化。

const W = 512;
const H = 512;

// ── 翅膀：在岗收着、超管张开、打盹下垂、警戒高举 ───────────────
const wings = (mode) => {
  const shapes = {
    rest: [
      'M232,196 C150,150 96,96 84,44 C130,44 196,74 232,132 Z',
      'M280,196 C362,150 416,96 428,44 C382,44 316,74 280,132 Z',
    ],
    spread: [
      'M230,190 C126,146 60,84 34,20 C104,16 196,60 236,120 Z',
      'M282,190 C386,146 452,84 478,20 C408,16 316,60 276,120 Z',
    ],
    droop: [
      'M234,206 C170,196 120,182 92,166 C136,138 196,146 238,176 Z',
      'M278,206 C342,196 392,182 420,166 C376,138 316,146 274,176 Z',
    ],
    alert: [
      'M226,182 C140,120 78,54 62,4 C132,10 214,52 240,110 Z',
      'M286,182 C372,120 434,54 450,4 C380,10 298,52 272,110 Z',
    ],
  }[mode];
  return `<g>
    ${shapes.map((d) => `<path d="${d}" fill="url(#wingGrad)" stroke="#9CC7DE" stroke-width="2.5"/>`).join('\n    ')}
    ${shapes.map((d) => `<path d="${d}" fill="none" stroke="#67E8F9" stroke-width="2" opacity="0.8"/>`).join('\n    ')}
  </g>`;
};

// ── 尾针：在岗收着，警戒时竖起并加粗 ──────────────────────────
const sting = (raised) =>
  raised
    ? `<path d="M256,398 L256,404 L246,436 L256,464 L266,436 L256,404 Z"
         fill="#E8890C" stroke="#2A1E16" stroke-width="3" stroke-linejoin="round"/>
       <circle cx="256" cy="464" r="6" fill="#2A1E16"/>`
    : `<path d="M256,406 L256,428" stroke="#2A1E16" stroke-width="7" stroke-linecap="round"/>`;

// ── 胸章：对勾（查验通过）/ 金星（超管）/ 感叹号（告警）────────
const badge = (kind) => {
  const hex = '<polygon points="256,206 282,221 282,251 256,266 230,251 230,221"';
  if (kind === 'star') {
    return `${hex} fill="#B45309" stroke="#FDE68A" stroke-width="2.5"/>
      <path d="M256,214 L262,230 L279,230 L265,240 L270,256 L256,246 L242,256 L247,240 L233,230 L250,230 Z"
            fill="#FDE68A"/>`;
  }
  if (kind === 'alert') {
    return `${hex} fill="#B91C1C" stroke="#FECACA" stroke-width="2.5"/>
      <path d="M256,216 L256,242" stroke="#FEF2F2" stroke-width="7" stroke-linecap="round"/>
      <circle cx="256" cy="254" r="4.5" fill="#FEF2F2"/>`;
  }
  return `${hex} fill="#0E7490" stroke="#67E8F9" stroke-width="2.5"/>
    <path d="M242,236 L252,247 L271,224" fill="none" stroke="#EAF5FC" stroke-width="5"
          stroke-linecap="round" stroke-linejoin="round"/>`;
};

// ── 表情：网格复眼保留（那是阿守的标志），只改眼形与嘴 ─────────
const face = (mood) => {
  const grid = (cx, cy, extra = '') => `
      <ellipse cx="${cx}" cy="${cy}" rx="17" ry="20" fill="#1F1A17"/>
      ${extra}
      <g stroke="#67E8F9" stroke-width="1.2" opacity="0.75">
        <path d="M${cx - 13},${cy - 10} H${cx + 13} M${cx - 13},${cy} H${cx + 13} M${cx - 13},${cy + 10} H${cx + 13}"/>
        <path d="M${cx - 7},${cy - 19} V${cy + 19} M${cx + 3},${cy - 19} V${cy + 19}"/>
      </g>`;

  if (mood === 'superuser') {
    // (＾▽＾) 开心弧眼 + 大笑
    return `
    <g stroke="#1F1A17" stroke-width="4" fill="none" stroke-linecap="round">
      <path d="M218,172 Q234,154 250,172"/>
      <path d="M262,172 Q278,154 294,172"/>
    </g>
    <path d="M236,196 Q256,216 276,196" fill="none" stroke="#2A1E16" stroke-width="4" stroke-linecap="round"/>
    <path d="M240,198 Q256,210 272,198" fill="#8A320A" opacity="0.35"/>`;
  }
  if (mood === 'dozing') {
    // (-_-) 闭眼 + 小嘴
    return `
    <g stroke="#1F1A17" stroke-width="4" fill="none" stroke-linecap="round">
      <path d="M217,170 Q234,178 251,170"/>
      <path d="M261,170 Q278,178 295,170"/>
    </g>
    <ellipse cx="256" cy="200" rx="7" ry="9" fill="#8A320A" opacity="0.85"/>`;
  }
  if (mood === 'alarmed') {
    // (#°益°) 竖眉 + 瞪眼 + 咬牙
    return `
    ${grid(234, 170)}
    ${grid(280, 170)}
    <g stroke="#1F1A17" stroke-width="4.5" fill="none" stroke-linecap="round">
      <path d="M216,148 L244,158"/>
      <path d="M296,148 L268,158"/>
    </g>
    <path d="M238,202 L274,202" stroke="#2A1E16" stroke-width="4" stroke-linecap="round"/>
    <g fill="#FEF2F2"><rect x="240" y="196" width="8" height="12" rx="1"/><rect x="264" y="196" width="8" height="12" rx="1"/></g>`;
  }
  if (mood === 'verifying') {
    // (・_・)? 一只眼瞪大 + 小圆嘴
    return `
    ${grid(234, 170)}
    <ellipse cx="281" cy="170" rx="19" ry="22" fill="#1F1A17"/>
    <g stroke="#67E8F9" stroke-width="1.2" opacity="0.75">
      <path d="M266,160 H296 M266,170 H296 M266,180 H296"/>
      <path d="M273,151 V189 M286,151 V189"/>
    </g>
    <ellipse cx="256" cy="202" rx="8" ry="8" fill="#8A320A" opacity="0.8"/>`;
  }
  // 值守（默认）：镇定微笑
  return `
    ${grid(234, 168)}
    ${grid(280, 168)}
    <path d="M244,200 Q256,210 268,200" fill="none" stroke="#2A1E16" stroke-width="3.5" stroke-linecap="round"/>`;
};

// ── 附加道具 / 氛围 ───────────────────────────────────────────
const extras = (mood) => {
  if (mood === 'verifying') {
    // 举着凭证（一张小卡片）+ 问号
    return `
  <g>
    <path d="M198,268 L150,300 L146,344" stroke="#2A1E16" stroke-width="6" fill="none" stroke-linecap="round"/>
    <path d="M150,300 L96,272" stroke="#2A1E16" stroke-width="6" fill="none" stroke-linecap="round"/>
    <rect x="52" y="228" width="52" height="66" rx="5" fill="#EAF5FC" stroke="#0E7490" stroke-width="3" transform="rotate(-12 78 261)"/>
    <g stroke="#0E7490" stroke-width="2.5" opacity="0.8" transform="rotate(-12 78 261)">
      <path d="M62,244 H94 M62,256 H94 M62,268 H86"/>
    </g>
  </g>
  <text x="330" y="132" font-family="system-ui, 'PingFang SC', sans-serif" font-size="44" font-weight="700"
        fill="#0E7490" opacity="0.9">?</text>`;
  }
  if (mood === 'dozing') {
    // Zzz
    return `
  <g fill="#0E7490" opacity="0.85" font-family="ui-monospace, Menlo, monospace" font-weight="700">
    <text x="332" y="150" font-size="26">z</text>
    <text x="352" y="124" font-size="20">z</text>
    <text x="368" y="102" font-size="15">z</text>
  </g>
  <path d="M120,96 A46,46 0 0 1 156,60 A40,40 0 0 0 108,80 Z" fill="#FDE68A" stroke="#C98A00" stroke-width="2"/>`;
  }
  if (mood === 'alarmed') {
    // 红晕告警光 + 一张写着 .env 的纸（被发现的密钥文件）
    return `
  <circle cx="256" cy="250" r="150" fill="#EF4444" opacity="0.10"/>
  <g transform="rotate(-10 96 150)">
    <rect x="62" y="120" width="66" height="80" rx="4" fill="#FFF7ED" stroke="#B91C1C" stroke-width="3"/>
    <text x="95" y="168" text-anchor="middle" font-family="ui-monospace, Menlo, monospace"
          font-size="20" font-weight="700" fill="#B91C1C">.env</text>
  </g>`;
  }
  if (mood === 'superuser') {
    // 超管光环 + 小皇冠
    return `
  <circle cx="256" cy="250" r="150" fill="#FDE68A" opacity="0.22"/>
  <path d="M214,96 L226,64 L244,84 L256,56 L268,84 L286,64 L298,96 Z"
        fill="#F59E0B" stroke="#8A320A" stroke-width="3" stroke-linejoin="round"/>`;
  }
  return '';
};

const svg = (mood, label) => `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${W} ${H}" width="${W}" height="${H}" role="img" aria-label="阿守（Keeper）· ${label}">
  <title>阿守（Keeper）· ${label}</title>
  <desc>BRD 项目宠物的「${label}」状态。</desc>
  <defs>
    <radialGradient id="bodyGrad" cx="45%" cy="30%" r="78%">
      <stop offset="0%" stop-color="#FFC62E"/><stop offset="68%" stop-color="#F5B301"/><stop offset="100%" stop-color="#DE9E00"/>
    </radialGradient>
    <radialGradient id="headGrad" cx="42%" cy="30%" r="80%">
      <stop offset="0%" stop-color="#FFE38A"/><stop offset="100%" stop-color="#F0AF00"/>
    </radialGradient>
    <linearGradient id="armorGrad" x1="0%" y1="0%" x2="0%" y2="100%">
      <stop offset="0%" stop-color="#22D3EE"/><stop offset="100%" stop-color="#0E7490"/>
    </linearGradient>
    <linearGradient id="wingGrad" x1="0%" y1="0%" x2="100%" y2="100%">
      <stop offset="0%" stop-color="#EAF5FC" stop-opacity="0.95"/><stop offset="100%" stop-color="#CFE8F5" stop-opacity="0.55"/>
    </linearGradient>
    <radialGradient id="glowGrad" cx="50%" cy="50%" r="50%">
      <stop offset="0%" stop-color="#67E8F9" stop-opacity="0.55"/><stop offset="100%" stop-color="#67E8F9" stop-opacity="0"/>
    </radialGradient>
  </defs>

  <ellipse cx="256" cy="452" rx="130" ry="18" fill="#1F1A17" opacity="0.12"/>
  <polygon points="256,336 342,386 342,462 256,492 170,462 170,386" fill="#FFF3C4" stroke="#C98A00" stroke-width="3"/>
  <polygon points="256,352 328,393 328,452 256,478 184,452 184,393" fill="#FFE38A" opacity="0.75"/>
  <text x="256" y="440" text-anchor="middle" font-family="ui-monospace, Menlo, Consolas, monospace"
        font-size="20" font-weight="700" fill="#8A320A">BRD</text>

  ${wings(mood === 'superuser' ? 'spread' : mood === 'dozing' ? 'droop' : mood === 'alarmed' ? 'alert' : 'rest')}
  ${extras(mood)}

  <g>
    <ellipse cx="256" cy="322" rx="76" ry="86" fill="url(#bodyGrad)" stroke="#2A1E16" stroke-width="3"/>
    <path d="M186,300 Q256,318 326,300" fill="none" stroke="#2A1E16" stroke-width="13" stroke-linecap="round"/>
    <path d="M182,342 Q256,362 330,342" fill="none" stroke="#2A1E16" stroke-width="13" stroke-linecap="round"/>
    <path d="M196,382 Q256,400 316,382" fill="none" stroke="#2A1E16" stroke-width="12" stroke-linecap="round"/>
    ${sting(mood === 'alarmed')}
  </g>

  <g>
    <ellipse cx="256" cy="238" rx="64" ry="54" fill="url(#bodyGrad)" stroke="#2A1E16" stroke-width="3"/>
    <path d="M192,238 Q256,268 320,238 L320,252 Q256,282 192,252 Z" fill="url(#armorGrad)" opacity="0.9"/>
    ${badge(mood === 'superuser' ? 'star' : mood === 'alarmed' ? 'alert' : 'check')}
  </g>

  <g stroke="#2A1E16" stroke-width="6" fill="none" stroke-linecap="round">
    <path d="M206,296 L166,336 L164,384"/>
    <path d="M214,322 L186,372 L190,414"/>
    <path d="M314,268 L362,300 L366,344"/>
    <path d="M306,322 L334,372 L330,414"/>
    ${mood === 'verifying' ? '' : '<path d="M198,268 L150,300 L146,344"/>'}
  </g>

  <g>
    <path d="M298,296 L352,330 L372,372" stroke="#2A1E16" stroke-width="6" fill="none" stroke-linecap="round"/>
    <circle cx="376" cy="384" r="19" fill="url(#glowGrad)"/>
    <circle cx="376" cy="382" r="13" fill="none" stroke="#8A320A" stroke-width="5"/>
    <g stroke="#8A320A" fill="none" stroke-linecap="round">
      <circle cx="362" cy="404" r="6.5" stroke-width="4" fill="#FFF3C4"/>
      <path d="M362,410 L362,442" stroke-width="5"/><path d="M362,432 L351,432" stroke-width="4.5"/>
      <circle cx="378" cy="410" r="6.5" stroke-width="4" fill="#FFF3C4"/>
      <path d="M378,416 L378,450" stroke-width="5"/><path d="M378,440 L390,440" stroke-width="4.5"/>
      <circle cx="394" cy="404" r="6.5" stroke-width="4" fill="#FFF3C4"/>
      <path d="M394,410 L394,440" stroke-width="5"/><path d="M394,430 L405,430" stroke-width="4.5"/>
    </g>
  </g>

  <g transform="${mood === 'dozing' ? 'rotate(7 256 190)' : ''}">
    <ellipse cx="256" cy="168" rx="52" ry="48" fill="url(#headGrad)" stroke="#2A1E16" stroke-width="3"/>
    <path d="M228,128 C214,104 200,94 182,88" stroke="#2A1E16" stroke-width="5" fill="none" stroke-linecap="round"/>
    <path d="M284,128 C298,104 312,94 330,88" stroke="#2A1E16" stroke-width="5" fill="none" stroke-linecap="round"/>
    <circle cx="180" cy="86" r="7" fill="#2A1E16"/>
    <circle cx="332" cy="86" r="7" fill="#2A1E16"/>
    ${face(mood)}
    <animateTransform attributeName="transform" type="translate" values="0,0; 0,-3; 0,0"
                      dur="${mood === 'dozing' ? '5s' : '2.4s'}" repeatCount="indefinite"/>
  </g>
</svg>
`;

const moods = [
  ['onduty', '值守', 'onDuty'],
  ['verifying', '查证', 'verifying'],
  ['superuser', '超管', 'superuser'],
  ['dozing', '打盹', 'dozing'],
  ['alarmed', '竖针', 'alarmed'],
];

import { writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
const here = dirname(fileURLToPath(import.meta.url));
for (const [slug, label] of moods) {
  const out = join(here, `keeper-${slug}.svg`);
  writeFileSync(out, svg(slug, label), 'utf8');
  console.log('written', `keeper-${slug}.svg`, `(${label})`);
}
