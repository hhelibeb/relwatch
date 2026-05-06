# RelWatch

GitHub Release 监控桌面应用，使用 DeepSeek 开发。

## 功能

| 功能 | 说明 |
|------|------|
| 自动轮询 | 定时检查监控源的版本更新 |
| 桌面通知 | 发现新版本时弹出系统通知 |
| AI 摘要 | 集成 DeepSeek API，自动生成版本更新摘要 |
| 系统托盘 | 关闭窗口时最小化到托盘，后台运行 |
| 代理支持 | 支持 HTTP 代理，AI 请求可独立配置 |
| 多语言 | 中文 / English |
| 安全存储 | GitHub Token、API Key 加密存储 |

## 技术栈

Tauri v2 + Rust + Vue 3 + TypeScript + SQLite

## 快速开始

```bash
npm install
npm run dev          # 开发模式
npm run tauri build  # 构建安装包
```
