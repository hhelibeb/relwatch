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
      ],
    },
  },
})
