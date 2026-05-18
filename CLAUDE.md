# Release Note 生成规则

生成简要的release note。当生成 release note 时，请遵循以下规则：

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
6. 务必注意：如果某项 fix 是为了解决本次 release 内其他 feat 引入的新问题（即该 bug 在上一版本中不存在），则不在 release note 中展示；相反，如果 fix 修复的是上个版本遗留的 bug，则需要正常列出
