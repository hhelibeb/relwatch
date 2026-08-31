---
name: release
description: 完整版本发布流程，覆盖版本号更新、本地验证、打 tag 推送、CI 轮询、Release Note 生成和发布。适用于 relwatch 项目发布新版本。
---

# Release 工作流

## 前置条件

确保在项目根目录执行，`gh` CLI 已登录，Git 已配置。

## 流程概览

```
Step 1  →  确认当前状态
Step 2  →  审查本次 Release 的 commits（提前到 tag 前）
Step 3  →  确定新版本号并更新版本文件
Step 4  →  更新 CHANGELOG.md（唯一数据源）
Step 5  →  本地完整验证
Step 6  →  提交 release bump
Step 7  →  tag 前最终确认
Step 8  →  打 tag 并精确推送
Step 9  →  等待 CI 并检查 Draft Release
Step 10 →  提取 Release Note 并更新 Draft Release
Step 11 →  等待用户确认
Step 12 →  发布并发布后检查
Step 13 →  回滚流程（仅失败时使用）
```

---

## Step 1：确认当前状态

确认以下条件：
- 当前在 `main` 分支
- 工作区干净（无未提交变更）
- `gh` CLI 已登录

```bash
# 确认 gh 已登录
gh auth status

# 获取上一个版本 tag
latest=$(git tag --sort=-v:refname | head -1)
echo "最新 tag: $latest"
```

同时记录以下信息：
- `<旧版本>`：上一个发布的版本号（如 `1.3.0`）
- `<新版本>`：本次要发布的版本号（还未更新）

---

## Step 2：审查本次 Release 的 commit message 和 diff

> ⚠️ 这一步**必须在**提交 release bump 之前完成，因为之后如果需要 rebase 修正 commit message，bump commit 会污染重排范围。

```bash
latest=$(git tag --sort=-v:refname | head -1)

# 查看变更范围
git log --oneline "$latest"..HEAD
echo "---"
git log --format='%h %s%n%b' "$latest"..HEAD
echo "---"
git diff --stat "$latest"..HEAD
```

审查要点：

- commit message 是否符合 Conventional Commit（`feat:` / `fix:` / `refactor:` / `chore:` / `docs:` / `style:`）
- 是否有明显分类错误（如 `refactor` 实际是新增功能）
- 是否有多个同类 commit 需要 squash
- 是否有不该进 release 的提交（调试代码、WIP）
- commit message 中是否有敏感信息

### 如果发现问题

在打 tag 前修正：

```bash
# 交互式 rebase 修改历史
latest=$(git tag --sort=-v:refname | head -1)
git rebase -i "$latest"
```

常用操作：
- `reword` — 修改 commit message
- `squash` / `fixup` — 合并 commit
- `drop` — 删除不需要的 commit

---

## Step 3：确定新版本号并更新版本文件

### 3.1 确定版本号

根据 Step 2 审查结果，按 SemVer 规则确定：

- **PATCH**（1.3.0 → 1.3.1）：bug 修复、小改动
- **MINOR**（1.3.0 → 1.4.0）：新功能、向后兼容
- **MAJOR**（1.3.0 → 2.0.0）：破坏性变更

```bash
echo "当前版本: $latest"
echo "目标版本: v<新版本>"
```

### 3.2 更新版本号文件

以下三个文件都需要更新：

1. `package.json` — `"version": "<新版本>"`
2. `src-tauri/Cargo.toml` — `version = "<新版本>"`
3. `src-tauri/tauri.conf.json` — `"version": "<新版本>"`

改完后快速确认：

```bash
echo "package.json:    $(node -p "require('./package.json').version")"
echo "Cargo.toml:      $(grep '^version' src-tauri/Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/')"
echo "tauri.conf.json: $(node -p "require('./src-tauri/tauri.conf.json').version")"
```

如果有 `package-lock.json`，同步更新：

```bash
npm install --package-lock-only
```

---

## Step 4：更新 CHANGELOG.md

> CHANGELOG 是 Release Note 的主数据源，后续 GitHub Release 正文直接从中提取，不独立生成。commit hash 从 git log v<旧版本>..v<新版本> 补充。

### 4.1 生成 [Unreleased] 内容

使用 changelog skill 更新 `[Unreleased]` 区块：基于 `$latest..HEAD` 的 commit 按 Conventional Commit 前缀分类（feat / fix / refactor / chore / docs），遵守 Keep a Changelog 1.1.0 格式。

```bash
# 调用全局 changelog skill 更新 [Unreleased] 区块
```

### 4.2 创建版本条目

1. 将 `[Unreleased]` 内容移到新版本条目 `## [<新版本>] - <YYYY-MM-DD>`
2. 清空 `[Unreleased]` 区块（保留标题，内容为空方便后续累积）
3. 在文件底部修改版本链接：
   - `[Unreleased]: https://github.com/hhelibeb/relwatch/compare/v<新版本>...HEAD`
   - 添加新行 `[<新版本>]: https://github.com/hhelibeb/relwatch/compare/v<旧版本>...v<新版本>`

---

## Step 5：本地完整验证

```bash
(
  set -euo pipefail

  # TypeScript 类型检查
  npx vue-tsc --noEmit

  # 前端测试
  npx vitest run

  # ESLint 检查
  npm run lint
)

(
  set -euo pipefail

  cd src-tauri

  # Rust 编译检查
  cargo check

  # Rust 全部测试
  cargo test

  # clippy 严格检查
  cargo clippy -- -D warnings
)
```

---

## Step 6：统一提交版本号和 CHANGELOG

```bash
git add package.json src-tauri/Cargo.toml src-tauri/tauri.conf.json CHANGELOG.md

# 如果有 package-lock.json，一起提交
git add package-lock.json 2>/dev/null || true

git commit -m "chore: bump version to v<新版本>"
```

版本号变更和 CHANGELOG 更新合并在一个 commit 中，后续 tag 将落在此 commit 上。

---

## Step 7：tag 前最终确认

打 tag 前强制检查以下条件：

```bash
# 1. 工作区干净
test -z "$(git status --porcelain)" || { echo "❌ 工作区不干净"; exit 1; }

# 2. 在 main 分支
test "$(git branch --show-current)" = "main" || { echo "❌ 不在 main"; exit 1; }

# 3. 版本号一致
pkg_ver=$(node -p "require('./package.json').version")
cargo_ver=$(grep '^version' src-tauri/Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/')
tauri_ver=$(node -p "require('./src-tauri/tauri.conf.json').version")
test "$pkg_ver" = "$cargo_ver" || { echo "❌ 版本号不一致"; exit 1; }
test "$pkg_ver" = "$tauri_ver" || { echo "❌ 版本号不一致"; exit 1; }

# 4. 当前 HEAD 是 release bump commit
echo "最近 3 个 commit:"
git log --oneline -3

# 5. 确认本次变更范围
echo "变更统计："
git diff --stat v<旧版本>..HEAD

echo "✅ 所有检查通过，可以打 tag"
```

---

## Step 8：打 tag 并精确推送

> ⚠️ 不要使用 `git push origin main --tags`，这会把本地所有未推送 tag 都推上去（包括测试 tag、遗留的 lightweight tag）。只推送当前需要的 tag。

```bash
git tag -a v<新版本> -m "v<新版本>"

git push origin main v<新版本>
```

---

## Step 9：等待 CI 并检查 Draft Release

### 9.1 等待 CI 全部通过

用 `./scripts/poll-ci.sh` 轮询，或手动检查。

需要同时检查两个 ref 的 workflow run：

```bash
# main 分支上的 CI / Lint / Secret Scan
gh run list --branch main --limit 10 --json name,status,conclusion

# tag 触发上的 Release workflow
gh run list --limit 20 --json name,status,conclusion,headBranch | grep "v<新版本>"
```

必须等待以下 workflow 全部 success：

| Workflow | 触发方式 | 检查命令 |
|----------|---------|---------|
| CI（frontend + backend tests） | push main | `gh run list --branch main` |
| Lint（clippy -D warnings） | push main | `gh run list --branch main` |
| Secret Scan | push main | `gh run list --branch main` |
| Release（构建产物 + draft release） | push tag v* | `gh run list --branch v<新版本>`（或在列表中按 tag 名筛选） |

### 9.2 检查 Draft Release 和产物

Release workflow（`tauri-action`）在推送 tag 后会自动创建 draft release。

```bash
# 查 draft release 信息
gh api repos/hhelibeb/relwatch/releases --paginate \
  --jq '.[] | select(.tag_name=="v<新版本>") | {id, tag_name, draft, name, body, assets: [.assets[].name]}'
```

确认：

- ✅ `draft` 为 `true`
- ✅ `tag_name` 是 `v<新版本>`
- ✅ assets 包含 Windows `.exe/.msi` 和 Linux `.deb`/`.AppImage`
- ✅ **应用内更新产物（v1.14.0 起）**：assets 包含 `latest.json` 与各平台 `.sig` 签名文件
  （`RelWatch_<v>_x64-setup.exe.sig` / `RelWatch_<v>_amd64.AppImage.sig` 等；deb 的 `.sig`
  是否生成以首次带签构建实测为准）。缺 `.sig`/`latest.json` 说明 CI 未注入
  `TAURI_SIGNING_PRIVATE_KEY*` secrets 或签名步骤被跳过（构建日志会有告警），判发布不合格。
- ✅ **latest.json 双平台断言**：下载 latest.json 并断言 `platforms` 同时含
  `windows-x86_64`（`-nsis`）与 `linux-x86_64`（`-appimage` / `-deb`）变体——
  tauri-action 的 latest.json 上传是「读旧 → 叠加本 job → 删旧传新」的非原子读改写，
  windows/ubuntu 双 job 并发时后写覆盖先写会产出单平台 latest.json，且发布"看起来"成功
  （另一平台用户永远收不到更新）：

  ```bash
  gh api repos/hhelibeb/relwatch/releases --paginate --jq '.[] | select(.tag_name=="v<新版本>") | .assets[] | select(.name=="latest.json") | .url' \
    | head -1 | xargs curl -sL | tee /tmp/latest.json
  # 断言：windows-x86_64 与 linux-x86_64 同时存在，缺任一 → 立即手动补传/重跑，不得发布
  node -e "const p=Object.keys(require('/tmp/latest.json').platforms); if(!p.some(k=>k.startsWith('windows-x86_64'))||!p.some(k=>k.startsWith('linux-x86_64'))){console.error('FATAL: platforms 缺平台:',p);process.exit(1)};console.log('platforms OK:',p.join(', '))"
  ```
- ⏸️ body 目前是 workflow 的固定文本 `请查看附件下载对应平台的安装包。`，后续会被替换

---

## Step 10：提取 Release Note 并更新 Draft Release

CHANGELOG 是内容数据源，但 Release Note 有独立的展示格式。从 CHANGELOG 中提取对应版本的条目后，按以下规则格式化成 Release Note。

### 10.1 提取原始内容

```bash
# 提取 v<新版本> 的 CHANGELOG 块到临时文件
awk '/^## \['"<新版本>"'\]/{flag=1; next} /^## \[/{flag=0} flag' CHANGELOG.md > /tmp/changelog-raw.md

cat /tmp/changelog-raw.md
```

### 10.2 格式化为 Release Note

基于提取的 CHANGELOG 内容和 git log，按以下规则生成 Release Note：

**分类映射规则：**

| CHANGELOG 分类 | Release Note 标题 |
|---------------|-------------------|
| `### Added` | 🚀 新功能 |
| `### Fixed` | 🐛 Bug 修复 |
| `### Changed` | 🔧 改进 |
| `### Removed` | 🗑️ 移除 |
| `### Deprecated` | ⏳ 即将弃用 |
| `### Security` | 🔒 安全更新 |
| `### Docs` / `### Style` | 💄 样式/文档优化 |

**格式要求：**

1. 每个条目标注对应的 commit hash（短格式），来自 `git log`：
   ```bash
   git log --oneline v<旧版本>..v<新版本> --grep="关键词"
   ```
2. 提取提交正文的核心信息，不要照搬标题
3. 使用**中文**
4. 如果某项分类没有对应内容，可以省略

**Fix 和 改进 范围规则：**

- 如果某个 fix 或 改进 是为了解决**本次 release 内**其他 feat 引入的新问题（即该 bug 在上一版本中不存在），则不作为独立条目展示，**但其 commit hash 应附加到所从属的 feature 条目末尾**（因为它是该功能完整交付的一部分）
- 如果 fix 或 改进 修复的是**上个版本遗留的 bug**，需要正常列出

**同类合并规则：**

- 同一 feature 的多个 commits（例如：一个 feat commit + 后续的修复/调整）合并到同一条目下，hash 全部附在末尾：`独立查看源。 (`feat123`, `fix456`)`
- 仅 housekeeping commits（chore bump、test-only、ci 配置）不出现

**⚠️ 常见陷阱：**

- 不要只看 CHANGELOG 标题分类，要对照 diff 确认实际变更性质
- 新功能归类到 🚀，重构/改进归类到 🔧，两者容易混淆
- 新增了图标/资源文件往往意味着新功能，不是改进

### 10.3 写入 Release Note 文件

```bash
# 将格式化后的 Release Note 写入文件
cat > .rpiv/artifacts/release-notes/v<新版本>.md << 'EOF'
🚀 新功能

（本次新增的功能列表...）

🐛 Bug 修复

（本次修复的 bug 列表...）

🔧 改进

（本次的改进列表...）
EOF

# 确认
cat .rpiv/artifacts/release-notes/v<新版本>.md
```

### 10.4 更新 Draft Release

```bash
gh release edit v<新版本> \
  --draft \
  --notes-file .rpiv/artifacts/release-notes/v<新版本>.md
```

如果后续需要再次更新 release note，重复执行上述命令即可。

---

## Step 11：等待用户确认

发布前必须由用户确认 release note 内容，用户说可以再发布。

---

## Step 12：发布并发布后检查

> **应用内更新时序（v1.14.0 起）**：`gh release edit --draft=false` 发布后，
> `releases/latest` 才会指向该版本，updater endpoint
> （`releases/latest/download/latest.json`）随之生效——draft 期间应用内「检查更新」
> 仍返回旧版本「已是最新」，属预期时序，不是故障。

```bash
# 发布
gh release edit v<新版本> --draft=false

# 发布后检查
gh release view v<新版本>
gh release view v<新版本> --json assets
git tag -n v<新版本>

# 可选：拉取最新状态
git fetch --tags origin
```

---

## Step 13：回滚流程

> 仅在 tag 打错、Release workflow 失败、或发现严重问题时使用。

### 场景一：tag 打错（尚未发布，仍是 draft）

```bash
# 删除远端 tag
git push origin :refs/tags/v<新版本>

# 删除本地 tag
git tag -d v<新版本>

# 删除 GitHub draft release（如果已创建）
gh release delete v<新版本> --yes
```

然后修正 commit，重新打 tag：

```bash
git tag -a v<新版本> -m "v<新版本>"
git push origin main v<新版本>
```

### 场景二：已发布但发现严重问题

如果 release 已经发布（非 draft），手动将 release 标记为 "Pre-release" 或直接在 GitHub 上删除：

```bash
# 删除 release（不删除 tag）
gh release delete v<新版本> --yes

# 如果也需要删除 tag
git push origin :refs/tags/v<新版本>
git tag -d v<新版本>
```

### 场景三：CHANGELOG 写错

在流程中任何阶段，如果发现 CHANGELOG 内容需要修正：

```bash
# 如果是 Step 7 之前——直接修改后重新提交
git add CHANGELOG.md
git commit --amend

# 如果是 Step 8 之后（已打 tag）但 Step 10 之前
# 需要先删除旧 tag，修正后重新打 tag（见场景一）
```

### ⚠️ 注意

- `tauri-action` 会在推送 tag 时创建 lightweight tag（`untagged-*`），删除旧 tag 并重打不会自动清理这些遗留 tag，需要手动清理
- 回滚后重新推送 tag 会触发新的 Release workflow，之前失败的 workflow 的 draft release 应当被新的覆盖（因为同名 tag）

---

## 常见陷阱总结

| 陷阱 | 说明 |
|------|------|
| Release Note 比较范围 | 必须用 `v<旧版本>..v<新版本>`，不要用 `v<新版本>..HEAD`（后者可能为空） |
| CI 双 ref 检查 | main 分支和 tag ref 对应不同的 workflow，两者都必须检查 |
