import { defineConfig } from 'vitest/config'
import vue from '@vitejs/plugin-vue'

export default defineConfig({
  plugins: [vue({
    template: {
      transformAssetUrls: false,
    },
  })],
  test: {
    environment: 'jsdom',
    globals: true,
    setupFiles: ['./src/__tests__/setup.ts'],
    // Node ≥25 默认在 globalThis 上预占 localStorage 访问器，vitest 会因此不再注入 jsdom 的实现；worker 也不继承主进程 execArgv，只能在这里传
    execArgv: 'localStorage' in globalThis ? ['--no-experimental-webstorage'] : [],
    coverage: {
      provider: 'v8',
      reporter: ['text', 'lcov', 'html'],
      include: ['src/**'],
      exclude: [
        'src/**/*.test.*',
        'src/__tests__/**',
        'src/vite-env.d.ts',
        'src/env.d.ts',
        'src/main.ts',
        'src/style.css',
        'src/components/releaseTypes.ts',
        // Tauri Specta 生成文件：不可控且不反映手写代码质量，不计入覆盖率门槛
        'src/bindings.ts',
      ],
      // 覆盖率回退防线：低于门槛时 vitest 以非零退出，CI 同步执行 pnpm run coverage
      thresholds: {
        statements: 75,
        branches: 70,
        functions: 72,
        lines: 78,
      },
    },
  },
})
