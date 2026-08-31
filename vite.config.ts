import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

export default defineConfig({
  plugins: [vue()],
  base: './',
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    watch: {
      // 排除整个 src-tauri：vite 只服务前端，Rust 变更由 tauri CLI 的
      // cargo watch 接管。否则 vite 会 watch target/ 下的编译产物，
      // Windows 上撞到正在被 rustc 写入锁定的 .pdb 会 EBUSY 崩溃
      // （"resource busy or locked, watch '...target\debug\deps\relwatch.pdb'"）。
      ignored: ['**/src-tauri/**'],
    },
  },
})
