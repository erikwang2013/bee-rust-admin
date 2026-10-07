import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import { visualizer } from 'rollup-plugin-visualizer';

// 体积分析：ANALYZE=1 pnpm build 额外产出 /tmp/bra-web-stats.{html,json}。
// 默认关闭——报告是给人看的产物，不进仓库也不影响正常构建。
// `json` 给 `scripts/top-modules.mjs` 读，`html` 给人在浏览器里点。
const analyze = process.env.ANALYZE === '1';

// 首屏真正用到的 antd 组件（布局 / 登录页 / 错误页 / 顶栏）—— 只有这些进 vendor 块。
//
// 「按包名整包列」是把重组件拖进首屏的元凶：`antd: ['antd']` 会把**任何页面用到的**
// antd 组件（Table / DatePicker / Tree…）都并进这个块，而这个块是 index.html 静态
// modulepreload 的，于是懒加载页面白拆了 —— 实测 rc-picker + antd/table 等约 1 MB
// （压缩前）就这么躺在首屏里。
//
// 现在只列下面这批：加进来的会连同依赖（rc-* 等）一起进 vendor 块；没列的（列表页
// 才用的 Table / DatePicker / Tree / Modal / Tabs / Descriptions…）不列，跟着各自的
// 懒加载页面走，首屏不再下载。**新增首屏组件时记得补进这个数组**，漏了不会坏，
// 只是它会被打进 entry 块（少了 vendor 缓存的好处）。
const ENTRY_ANTD = [
  'antd/es/app',
  'antd/es/avatar',
  'antd/es/badge',
  'antd/es/breadcrumb',
  'antd/es/button',
  'antd/es/card',
  'antd/es/config-provider',
  'antd/es/dropdown',
  'antd/es/empty',
  'antd/es/form',
  'antd/es/input',
  'antd/es/layout',
  'antd/es/locale/en_US',
  'antd/es/locale/zh_CN',
  'antd/es/menu',
  'antd/es/message',
  'antd/es/popover',
  'antd/es/result',
  'antd/es/select',
  'antd/es/space',
  'antd/es/spin',
  'antd/es/switch',
  'antd/es/theme',
  'antd/es/typography',
];

export default defineConfig({
  plugins: [
    react(),
    ...(analyze
      ? [
          visualizer({ filename: '/tmp/bra-web-stats.html', template: 'treemap' }),
          visualizer({ filename: '/tmp/bra-web-stats.json', template: 'raw-data' }),
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
          antd: ENTRY_ANTD,
        },
      },
    },
  },
});
