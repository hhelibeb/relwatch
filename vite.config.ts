import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

export default defineConfig({
  plugins: [vue()],
  base: './',
  clearScreen: false,
  build: {
    // 体积警告阈值放宽：默认 500 kB 是给 Web 算的（首屏要下载），而产物由
    // Tauri 从本地磁盘经自定义协议加载，没有下载成本，只剩解析开销。
    // 放宽后这警告仍然只会在真进了重量级依赖（整包拖进主 chunk）时才响——
    // 本地加载没有网络延迟会提醒你，它是最后一道信号。
    chunkSizeWarningLimit: 800,
  },
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
