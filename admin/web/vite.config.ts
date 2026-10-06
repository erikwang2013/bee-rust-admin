import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import { visualizer } from 'rollup-plugin-visualizer';

// 体积分析：ANALYZE=1 pnpm build 额外产出 /tmp/brd-web-stats.{html,json}。
// 默认关闭——报告是给人看的产物，不进仓库也不影响正常构建。
// `json` 给 `scripts/top-modules.mjs` 读，`html` 给人在浏览器里点。
const analyze = process.env.ANALYZE === '1';

export default defineConfig({
  plugins: [
    react(),
    ...(analyze
      ? [
          visualizer({ filename: '/tmp/brd-web-stats.html', template: 'treemap' }),
          visualizer({ filename: '/tmp/brd-web-stats.json', template: 'raw-data' }),
        ]
      : []),
  ],
  server: {
    port: 5173,
    proxy: {
      '/api': { target: 'http://127.0.0.1:8080', changeOrigin: true },
    },
  },
  // 生产构建的本地预览（pnpm preview）：端口与线上 nginx 一致，/api 同样反代到后端，
  // 这样不装 nginx 也能在本机跑通完整前后端链路
  preview: {
    port: 8081,
    proxy: {
      '/api': { target: 'http://127.0.0.1:8080', changeOrigin: true },
    },
  },
  build: {
    outDir: 'dist',
    sourcemap: false,
    rollupOptions: {
      output: {
        // 框架与组件库单独成块：业务代码改动不会让用户重下这两坨（内容哈希不变即可命中缓存）
        //
        // 这里**不能**再列 '@ant-design/icons'：数组写法会把这个包连同它的整棵依赖树
        // （836 个图标模块）全部并进 antd 块，而 antd 块是首屏静态加载的 —— 那样
        // src/icons.ts 里的按需引入就白做了。antd 自己用到的图标会由依赖关系自然
        // 并入 antd 块，剩下没用到的归入懒加载块，只有运维填了映射外的 icon 名才下载。
        manualChunks: {
          react: ['react', 'react-dom', 'react-router'],
          antd: ['antd'],
        },
      },
    },
  },
});
