---
name: release
description: 完整版本发布流程，覆盖版本号更新、本地验证、打 tag 推送、CI 轮询、Release Note 生成和发布。适用于 relwatch 项目发布新版本。
---

# Release 工作流

## 前置条件
确保在项目根目录执行，`gh` CLI 已登录，Git 已配置。

## Step 1：确认改动范围

```bash
latest=$(git tag --sort=-v:refname | head -1)
git log --oneline "$latest"..HEAD
git diff --stat "$latest"..HEAD
```

## Step 2：更新版本号

先找出所有需要改版本号的文件：

```bash
grep '"version"' package.json src-tauri/tauri.conf.json
grep '^version' src-tauri/Cargo.toml
```

三个文件都必须改：
- `package.json`
- `src-tauri/Cargo.toml`
- `src-tauri/tauri.conf.json`

## Step 3：本地完整验证

```bash
# 前端测试
npm test

# ESLint 检查（必须！零错误零警告）
npm run lint

# Rust 编译检查
cd src-tauri && cargo check

# Rust 全部测试
cargo test

# clippy 严格检查（必须！容易漏）
cargo clippy -- -D warnings

cd ..
```

## Step 4：提交版本号变更

```bash
git add package.json src-tauri/Cargo.toml src-tauri/tauri.conf.json
git commit -m "chore: bump version to v<新版本>"
```

## Step 5：确认 commit message 准确

审查 diff 中的**实际代码变更**，确保 commit message 准确反映变动性质：
- 新增文件/功能 → `feat:`，release note 归入 🚀 新功能
- 修复已有问题 → `fix:`，release note 归入 🐛 Bug 修复
- 移动/重命名/提取代码 → `refactor:`，release note 归入 🔧 重构
- 不要只抄标题，要确认是**新增**还是**重构**

## Step 6：打 tag 并推送

```bash
# tag 必须在所有 fix commit 合并之后打，不能提前
git tag -a v<新版本> -m "v<新版本>"
git push origin main --tags
```

## Step 7：等待 CI 全部通过

用 `./scripts/poll-ci.sh` 轮询，必须等待以下 workflow 全部 success：
- CI（frontend + backend tests）
- Lint（clippy -D warnings）
- Secret Scan
- Release（tag workflow，构建产物 + draft release）

## Step 8：检查 Release 产物

```bash
# 取 GitHub token（API 要认证才能看到 draft）
TOKEN=$(git credential fill <<<"protocol=https\nhost=github.com\n" 2>/dev/null | grep password | cut -d= -f2 | head -1)

# 确认 draft release 存在，且 3 个 artifact 已上传
curl -s -H "Authorization: token $TOKEN" \
  "https://api.github.com/repos/hhelibeb/relwatch/releases?per_page=100"
```

确认：
- tag_name 正确
- body 为空（等待 release note）
- assets 包含 exe / deb / AppImage

## Step 9：生成并更新 Release Note

**生成规则：**
1. 使用 `git tag --sort=-v:refname` 找最新 tag，对比 `tag..HEAD`
2. 按 Conventional Commit 前缀分类，如果某项分类没有对应内容，可以省略：
   - feat → 🚀 新功能
   - fix → 🐛 Bug 修复
   - style/docs → 💄 样式/文档优化
   - refactor → 🔧 重构
   - chore → 📦 其他/杂项
3. 提取提交正文的核心信息，不要照搬标题
4. 每个条目标注 commit hash（短格式）
5. 使用中文
6. 如果某项 fix 是为了解决本次 release 内其他 feat 引入的新问题（即该 bug 在上一版本中不存在），则不在 release note 中展示；相反，如果 fix 修复的是上个版本遗留的 bug，则需要正常列出

**⚠️ 常见陷阱：**
- 不要只看 commit message 标题分类 release note，要对照 diff 确认实际变更性质
- 新功能归类到 🚀，重构归类到 🔧，两者容易混淆
- 新增了图标/资源文件往往意味着新功能，不是重构

## Step 10：更新 Draft Release

```bash
TOKEN=$(git credential fill <<<"protocol=https\nhost=github.com\n" 2>/dev/null | grep password | cut -d= -f2 | head -1)

curl -s -X PATCH -H "Authorization: token $TOKEN" \
  -H "Accept: application/vnd.github+json" \
  "https://api.github.com/repos/hhelibeb/relwatch/releases/<RELEASE_ID>" \
  -d '{"body": "<release note 内容>"}'
```

## Step 11：等待用户确认

发布前必须由用户确认 release note 内容，用户说可以再发布。

## Step 12：发布

在 GitHub Releases 页面点击 Publish，或通过 API：

```bash
TOKEN=$(git credential fill <<<"protocol=https\nhost=github.com\n" 2>/dev/null | grep password | cut -d= -f2 | head -1)

curl -s -X PATCH -H "Authorization: token $TOKEN" \
  "https://api.github.com/repos/hhelibeb/relwatch/releases/<RELEASE_ID>" \
  -d '{"draft": false}'
```
