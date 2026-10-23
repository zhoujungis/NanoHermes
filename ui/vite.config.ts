import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

// Frontend project root. `npm run build` writes `dist/` here, which is what
// `gui/tauri.conf.json` ships as its `frontendDist`.
export default defineConfig(({ mode }) => {
  // 外部 shell 若导出了 NODE_ENV=production，Vitest 会沿用它：react 会解析到生产构建，
  // 而生产构建不导出 `act`，@testing-library/react 的每次 render 都会抛
  // "React.act is not a function"。测试进程必须固定为 test 环境，不能依赖调用方的 shell。
  if (mode === "test") process.env.NODE_ENV = "test";

  return {
    plugins: [react()],
    clearScreen: false,
    server: {
      port: 1420,
      strictPort: true,
      watch: { ignored: ["**/dist/**"] },
    },
    build: {
      outDir: "dist",
      emptyOutDir: true,
      target: "chrome120",
      sourcemap: false,
      rollupOptions: {
        output: {
          // highlight.js（core + 按需注册的语言）单独成块，其余依赖归入 vendor，
          // 各分块并行拉取、独立缓存，主 chunk 不再顶到 500kB 警戒线。
          manualChunks(id: string) {
            if (id.includes("node_modules/highlight.js")) return "highlight";
            if (id.includes("node_modules")) return "vendor";
            return undefined;
          },
        },
      },
    },
    test: {
      // 组件测试需要 DOM；纯逻辑测试在同一个环境里跑没有副作用。
      environment: "jsdom",
      globals: true,
      setupFiles: ["./src/test/setup.ts"],
      include: ["src/**/*.test.{ts,tsx}"],
      restoreMocks: true,
    },
  };
});
