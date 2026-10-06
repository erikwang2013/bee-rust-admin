#!/usr/bin/env node
// 体积报告：读 rollup-plugin-visualizer 的 raw-data 产物，按 chunk 列模块体积。
//
//   ANALYZE=1 pnpm build && node scripts/top-modules.mjs [chunk 关键字] [N]
//
// 不带参数列出所有 chunk（按体积降序）；带关键字（如 `antd`）只列该 chunk 的前 N 个模块。
// 产物在 /tmp，不进仓库。`renderedLength` 是 chunk 内的实际字节数（压缩后）。
import { readFileSync } from 'node:fs';

const stats = JSON.parse(readFileSync('/tmp/brd-web-stats.json', 'utf8'));
const [, , filter, nArg] = process.argv;
const N = Number(nArg) || 10;

// tree 是目录树：叶子节点带 uid，体积在 nodeParts[uid].renderedLength
function collect(node, prefix = '', out = []) {
  const path = prefix ? `${prefix}/${node.name}` : node.name;
  if (node.uid) out.push({ name: path, bytes: stats.nodeParts[node.uid]?.renderedLength ?? 0 });
  for (const c of node.children ?? []) collect(c, path, out);
  return out;
}

const chunks = stats.tree.children
  .map((c) => {
    const mods = collect(c).sort((a, b) => b.bytes - a.bytes);
    return { name: c.name, mods, bytes: mods.reduce((a, m) => a + m.bytes, 0) };
  })
  .sort((a, b) => b.bytes - a.bytes);

const picked = filter ? chunks.filter((c) => c.name.includes(filter)) : chunks;
for (const chunk of picked) {
  console.log(`== ${chunk.name}  ${(chunk.bytes / 1024).toFixed(1)} kB`);
  if (!filter) continue; // 总览模式只列 chunk 体积
  for (const m of chunk.mods.slice(0, N)) {
    const short = m.name.replace(/^.*\/node_modules\/(\.pnpm\/[^/]+\/node_modules\/)?/, '');
    console.log(`  ${(m.bytes / 1024).toFixed(1).padStart(8)} kB  ${short}`);
  }
  console.log();
}
