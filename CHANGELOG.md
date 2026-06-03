# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.3.3] - 2026-06-03

### Added
- Release 右键菜单支持删除功能。

### Changed
- 加密系统升级：OS keyring 托管 master key，支持 v1→v2 密文自动迁移，避免无限重试。
- Release 获取支持完整分页拉取：fetch_history_count=0 获取全部历史版本，上限从 50 提升到 100。
- i18n 渲染引擎重构为 OnceLock 惰性静态初始化，提取辅助函数。
- i18n 系统基础打磨：添加 JSDoc、类型保护、语言回退修复。
- 日志渲染现在使用所选语言而非硬编码的 zh-CN。

### Fixed
- Windows 凭据管理器持久化修复：使用 CRED_PERSIST_LOCAL_MACHINE 解决非域环境重启后凭据丢失问题。
- 右键菜单打开新菜单时关闭之前已打开的菜单，防止多个菜单重叠。
- 修复数据库并发操作及通知回调中的 panic 问题。

## [1.3.1] - 2026-06-01

### Fixed
- 重新启用被自动禁用的监控源后不再因累积的失败计数而被立即再次禁用。
- 5xx 服务器错误、网络错误、401 认证错误和 429 限流错误统一归为 WARN 级别，避免 ERROR 告警泛滥。
- 断路器自动禁用监控源的日志文案与手动暂停区分，避免用户混淆。
- 监控源因连续失败被自动禁用时发送桌面通知。
- GitHub API 错误提示统一使用国际化键格式，支持多语言翻译。
- 切换语言后输入框右键菜单标签即时更新。

## [1.3.0] - 2026-05-31

### Added
- Log entry search with i18n-rendered message field for keyword matching.
- Log level filtering (INFO / WARN / ERROR) with enhanced error level classification for different failure scenarios.
- Confirmation dialog before clearing all logs.
- Right-click context menus for input fields (cut/copy/paste/select all) and AI summary text (copy).
- Automatic source disabling after 3 consecutive fetch failures (circuit breaker); HTTP status codes are propagated into error logs for better diagnostics.

### Fixed
- Discard button now correctly restores the language and theme live preview.
- Discard changes only resets preview when language or theme has unsaved changes.
- Release search bar TypeScript type error.
- Browser keyboard shortcuts (Ctrl+S, F5, F12, etc.) no longer interfere with the application.

## [1.2.4] - 2026-04-27

### Fixed
- Restored DEB packaging target for Linux builds.

## [1.2.3] - 2026-04-24

### Added
- Unsaved changes indicator: dirty-mark banner, field-level blue dots, and sidebar group blue dots.

### Changed
- Reorganized settings page — language and tray options moved to display settings; language selector now shows hover preview.

## [1.2.2] - 2026-04-20

### Added
- Proxy mode selection in settings (none / system proxy / custom proxy).
- Version filter button on monitor sources; unread links sync with filter state.
- Consolidated retry count and intermediate version smart notifications.

### Fixed
- Theme preview toggle inversion, tray red dot not updating after single-source check, AI summary retries, and ReleaseItem navigation order.

## [1.2.1] - 2026-04-16

### Fixed
- Synchronized `tauri.conf.json` version number to match `package.json` and `Cargo.toml`.

## [1.2.0] - 2026-04-13

### Added
- Tauri build script for CI Release workflow integration.

## [1.1.4] - 2026-04-10

### Added
- Various improvements and fixes.

## [1.1.3] - 2026-04-08

### Added
- Backup import confirmation dialog.
- Immediate poll on startup when `next_poll_at` has expired.
- Exported backup filename now includes timestamp and hostname.
- Success operations logged to the activity log.
- Token hint moved to UI description area.

### Changed
- Improved toast notification copy.

## [1.1.2] - 2026-04-06

### Added
- Light/Dark theme toggle.

## [1.1.1] - 2026-04-04

### Fixed
- Owner/repo format matching in search input.

## [1.1.0] - 2026-04-01

### Added
- Source health status indicators.
- Version status management (read/unread/ignored).
- System tray enhancements.

## [1.0.6] - 2026-03-28

### Added
- Calendar view now prevents navigation to future months.
- GitHub icon on settings page.

### Changed
- Adjusted window and calendar proportions.

## [1.0.5] - 2026-03-25

### Added
- Three release list view modes: Simple, Aggregated, and Calendar.

## [1.0.4] - 2026-03-22

### Added
- Concurrent GitHub Release queries with retry mechanism.
- Toast notification for new version availability.

## [1.0.3] - 2026-03-19

### Changed
- Tab styling optimization and spacing unification.

[Unreleased]: https://github.com/hhelibeb/relwatch/compare/v1.3.3...HEAD
[1.3.3]: https://github.com/hhelibeb/relwatch/compare/v1.3.1...v1.3.3
[1.3.1]: https://github.com/hhelibeb/relwatch/compare/v1.3.0...v1.3.1
[1.3.0]: https://github.com/hhelibeb/relwatch/compare/v1.2.6...v1.3.0
[1.2.4]: https://github.com/hhelibeb/relwatch/compare/v1.2.3...v1.2.4
[1.2.3]: https://github.com/hhelibeb/relwatch/compare/v1.2.2...v1.2.3
[1.2.2]: https://github.com/hhelibeb/relwatch/compare/v1.2.1...v1.2.2
[1.2.1]: https://github.com/hhelibeb/relwatch/compare/v1.2.0...v1.2.1
[1.2.0]: https://github.com/hhelibeb/relwatch/compare/v1.1.4...v1.2.0
[1.1.4]: https://github.com/hhelibeb/relwatch/compare/v1.1.3...v1.1.4
[1.1.3]: https://github.com/hhelibeb/relwatch/compare/v1.1.2...v1.1.3
[1.1.2]: https://github.com/hhelibeb/relwatch/compare/v1.1.1...v1.1.2
[1.1.1]: https://github.com/hhelibeb/relwatch/compare/v1.1.0...v1.1.1
[1.1.0]: https://github.com/hhelibeb/relwatch/compare/v1.0.6...v1.1.0
[1.0.6]: https://github.com/hhelibeb/relwatch/compare/v1.0.5...v1.0.6
[1.0.5]: https://github.com/hhelibeb/relwatch/compare/v1.0.4...v1.0.5
[1.0.4]: https://github.com/hhelibeb/relwatch/compare/v1.0.3...v1.0.4
[1.0.3]: https://github.com/hhelibeb/relwatch/compare/v1.0.2...v1.0.3
