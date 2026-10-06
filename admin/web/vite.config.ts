import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

export default defineConfig({
  plugins: [react()],
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
        manualChunks: {
          react: ['react', 'react-dom', 'react-router'],
          antd: ['antd', '@ant-design/icons'],
        },
      },
    },
  },
});
