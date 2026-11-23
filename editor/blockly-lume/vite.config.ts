import { defineConfig, loadEnv } from 'vite';

// 所有资源都从 src 导入，避免构建时复制 publicDir。
// VITE_BASE_URL 由 CI 设置为 /LumeC/，本地开发默认从根路径访问。
export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, '.', 'VITE_');
  return {
    publicDir: false,
    base: env.VITE_BASE_URL || '/',
  };
});
