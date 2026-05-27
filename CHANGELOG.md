# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Log tab keyword search with pagination (50 entries per page).
- Customizable AI prompt for release summary generation.
- Configurable notification threshold to control alert frequency.
- `deepseek_proxy_bypass` setting to route AI API calls directly, bypassing the system proxy.

### Changed
- AI settings UI now shows conditionally based on enabled features.

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

[Unreleased]: https://github.com/hhelibeb/relwatch/compare/v1.2.4...HEAD
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
