/**
 * 源类型注册表：把散落在各组件里的 source_type 特判（URL 模板、图标、标题、
 * 显示名、解析规则、展示行为）收敛为每类型一条定义。
 *
 * 新增监控源类型的完整清单：
 * 1. 后端 `src-tauri/src/source.rs::get_adapter` + 新 adapter 文件
 * 2. 本文件加一条 `SourceTypeDef`（解析器/URL/图标/标题/展示行为）
 * 3. i18n 两个语言文件加 `source.type_<type>` key
 * 4. 若类型需要独立鉴权，后端 `AuthKind` 加枚举并注册 settings token
 */
import type { Source } from './sources'
import type { SourceType } from '../components/releaseTypes'

/** 输入解析结果：type 与后端 source_type 一致。 */
export interface ParsedSource {
  type: string
  owner: string
  repo: string
}

/** 源类型结构化元数据视图（HF：pipeline_tag / downloads / likes / gated）。 */
export interface HfMetaView {
  pipeline_tag: string | null
  downloads: number | null
  likes: number | null
  gated: boolean | null
}

/** 解析 GitHub 仓库输入，支持链接或 `owner/repo` 短格式。 */
export function parseGitHubUrl(raw: string): { owner: string; repo: string } | null {
  const input = raw.trim()
  const urlMatch = input.match(/github\.com\/([^/]+)\/([^/?#]+)/)
  if (urlMatch) return { owner: urlMatch[1], repo: urlMatch[2] }
  if (input.includes('github.com')) return null
  const repoMatch = input.match(/^([a-zA-Z0-9][a-zA-Z0-9_.-]*)\/([a-zA-Z0-9_.-]+)$/)
  if (repoMatch) return { owner: repoMatch[1], repo: repoMatch[2] }
  return null
}

/** 解析 HuggingFace 组织输入，支持组织名或 huggingface.co/<org> 链接。 */
export function parseHFOrgUrl(raw: string): string | null {
  const input = raw.trim()
  // https://huggingface.co/organizations/moonshotai/ 或带尾随路径
  const orgMatch = input.match(/huggingface\.co\/organizations\/([a-zA-Z0-9_-]+)/)
  if (orgMatch) return orgMatch[1]
  // https://huggingface.co/moonshotai（但排除 /datasets、/spaces 等保留路径）
  const urlMatch = input.match(/huggingface\.co\/([a-zA-Z0-9_-]+)/)
  if (urlMatch && !['datasets', 'spaces', 'models', 'org', 'organizations', 'settings', 'login'].includes(urlMatch[1])) {
    return urlMatch[1]
  }
  if (input.includes('huggingface.co')) return null
  // 已经是组织名
  if (/^[a-zA-Z][a-zA-Z0-9_-]*$/.test(input)) return input
  return null
}

/** 解析 YouTube 频道输入，返回 owner（channel_id 或 @handle）。 */
export function parseYoutubeUrl(raw: string): string | null {
  let input = raw.trim()
  // 解码 URL 编码（如 @%E6%81%8B%E4%B8%8A%E9%BB%98%E7%99%BD → @恋上默白）
  try {
    if (input.includes('%')) input = decodeURIComponent(input)
  } catch {
    // 非法转义序列时保持原样，由后续规则判定
  }
  // 已是 channel_id（UC + 22 位 base64 字符）
  if (/^UC[a-zA-Z0-9_-]{10,}$/.test(input)) return input
  // 以 UC 开头但长度不足：不是合法 channel_id，也不应视为 handle
  if (/^UC/.test(input)) return null
  // 纯 handle（@xxx 或 xxx，支持中文等 Unicode 字符）
  if (/^@?[\p{L}\p{N}_.-]{3,30}$/u.test(input)) {
    return input.startsWith('@') ? input : `@${input}`
  }
  // https://www.youtube.com/channel/UCxxx / youtube.com/channel/UCxxx
  const channelMatch = input.match(/(?:youtube\.com)\/channel\/(UC[a-zA-Z0-9_-]+)/)
  if (channelMatch) return channelMatch[1]
  // @handle 链接（youtube.com/@handle，支持 Unicode）
  const linkMatch = input.match(/(?:youtube\.com)\/@([^/?#]{1,50})/)
  if (linkMatch) return `@${linkMatch[1]}`
  const customMatch = input.match(/(?:youtube\.com)\/(?:c|user)\/([^/?#]{1,50})/)
  if (customMatch) return `@${customMatch[1]}`
  return null
}

/** 解析 B 站 UP 主输入，返回 UID：纯数字 UID / space.bilibili.com 链接。
 * 注意：B 站新注册用户使用 16 位新式 UID（space.bilibili.com/{mid} 的 mid 即 UID）。 */
export function parseBilibiliUrl(raw: string): string | null {
  const input = raw.trim()
  // 纯数字 UID（2~16 位：旧式 6~10 位、新式 16 位）
  if (/^\d{2,16}$/.test(input)) return input
  // space.bilibili.com/{uid} / bilibili.com/space/{uid}
  const spaceMatch = input.match(/(?:space\.bilibili\.com|bilibili\.com\/space)\/(\d+)/)
  if (spaceMatch) return spaceMatch[1]
  // bilibili.com/{uid} 跳转形式（排除 video/space 等保留路径）
  const biliMatch = input.match(/bilibili\.com\/(\d+)/)
  if (biliMatch) return biliMatch[1]
  return null
}

/** 去掉旧版 "YouTube channel: " 前缀。 */
function stripYtChannelPrefix(d: string): string {
  return d.startsWith('YouTube channel: ') ? d.slice('YouTube channel: '.length) : d
}

/**
 * 源类型定义。新增类型在此登记一条，组件按 type 查表即可，
 * 不再散落 `source_type === 'xxx'` 特判。
 */
export interface SourceTypeDef {
  /** 与后端 source_type 一致的标识。 */
  type: SourceType
  /** 类型标题 i18n key（如 source.type_github）。 */
  titleKey: string
  /** 源徽标图标 href。 */
  icon: string
  /** 源主页 URL。 */
  homeUrl: (owner: string, repo: string) => string
  /** 输入解析（链接与启发式共用），返回 null 表示不匹配。 */
  parse: (input: string) => { owner: string; repo: string } | null
  /** 明确特征匹配（域名 / @handle / 前缀），命中即优先尝试该类型。 */
  matches?: (input: string) => boolean
  /** 无特征纯文本的兜底解析（按数组顺序尝试，如 github 短格式 → HF 组织名）。 */
  fallback?: boolean
  /** 源显示名（默认 owner/repo，repo 空则 owner）。 */
  displayName?: (owner: string, repo: string, description: string | null) => string
  /** 跳转版本列表的搜索关键词（默认 owner/repo）。 */
  searchQuery?: (owner: string, repo: string, description: string | null) => string
  /** 添加源时是否展示专属配置 UI（如 YouTube 订阅复选框）。 */
  hasConfigInput?: boolean
  /** 是否参与 AI 摘要/翻译（false 则隐藏翻译入口；与后端 ai_eligible 对应）。 */
  aiSummary?: boolean
  /** 列表卡片是否显示 owner/repo 前缀行（HF tag 已含组织名、YT 显示频道名）。 */
  showRepoPrefix?: boolean
  /** 详情弹窗是否显示 owner/repo 前缀（HF tag 已含组织名；默认 true）。 */
  showRepoInDetail?: boolean
  /** 是否显示 tag（YouTube videoId 无意义；默认 true）。 */
  showTag?: boolean
  /** YouTube 风格封面布局（封面 + 标题），并启用 youtubeMeta 解析。 */
  youtubeLayout?: boolean
  /** 结构化元数据渲染（HF：pipeline_tag / downloads / likes / gated）。 */
  renderMeta?: (release: { source_type: string; extra_metadata: string | null }) => HfMetaView | null
}

export const sourceTypeDefs: SourceTypeDef[] = [
  {
    type: 'github',
    titleKey: 'source.type_github',
    icon: '/icons.svg#github-mark',
    homeUrl: (owner, repo) => `https://github.com/${owner}/${repo}`,
    parse: parseGitHubUrl,
    matches: input => input.includes('github.com'),
    // owner/repo 短格式兜底（如 microsoft/vscode）
    fallback: true,
    // GitHub 仓库名/组织名是标准形态，列表卡片显示 owner/repo 前缀
    showRepoPrefix: true,
  },
  {
    type: 'huggingface',
    titleKey: 'source.type_huggingface',
    icon: '/icons.svg#huggingface-icon',
    homeUrl: owner => `https://huggingface.co/${owner}`,
    parse: input => {
      const org = parseHFOrgUrl(input)
      return org ? { owner: org, repo: '' } : null
    },
    matches: input => input.includes('huggingface.co'),
    // 纯组织名兜底（顺序在 github 之后，单 token 文本如 moonshotai）
    fallback: true,
    // tag_name 已含组织名（moonshotai/Kimi），详情弹窗不重复显示前缀
    showRepoInDetail: false,
    renderMeta: release => {
      if (!release.extra_metadata) return null
      try {
        const obj = JSON.parse(release.extra_metadata)
        return {
          pipeline_tag: typeof obj.pipeline_tag === 'string' ? obj.pipeline_tag : null,
          downloads: typeof obj.downloads === 'number' ? obj.downloads : null,
          likes: typeof obj.likes === 'number' ? obj.likes : null,
          gated: typeof obj.gated === 'boolean' ? obj.gated : null,
        }
      } catch {
        return null
      }
    },
  },
  {
    type: 'youtube',
    titleKey: 'source.type_youtube',
    icon: '/icons.svg#youtube-icon',
    homeUrl: owner => `https://www.youtube.com/channel/${owner}`,
    parse: input => {
      const owner = parseYoutubeUrl(input)
      return owner ? { owner, repo: '' } : null
    },
    matches: input =>
      input.includes('youtube.com') ||
      input.includes('youtu.be') ||
      input.startsWith('@') ||
      /^UC[a-zA-Z0-9_-]{10,}$/.test(input),
    // 频道名（description，channel_id 无阅读意义），兼容旧版前缀
    displayName: (owner, _repo, description) => {
      const d = (description ?? '').trim()
      return stripYtChannelPrefix(d) || owner
    },
    // 跳转版本列表用频道名搜索（channel_id 用户不可读）
    searchQuery: (owner, _repo, description) => {
      const d = (description ?? '').trim()
      return stripYtChannelPrefix(d) || owner
    },
    // 添加时展示订阅内容复选框（视频/直播）
    hasConfigInput: true,
    // 视频不生成 AI 摘要/翻译（与后端 ai_eligible=false 对应）
    aiSummary: false,
    // tag 是 videoId，隐藏；显示频道名
    showTag: false,
    // B 站风格封面布局
    youtubeLayout: true,
  },
  {
    type: 'bilibili',
    titleKey: 'source.type_bilibili',
    icon: '/icons.svg#bilibili-icon',
    homeUrl: owner => `https://space.bilibili.com/${owner}`,
    parse: input => {
      const uid = parseBilibiliUrl(input)
      return uid ? { owner: uid, repo: '' } : null
    },
    matches: input => input.includes('bilibili.com') || /^\d{2,16}$/.test(input),
    // 显示 UP 主名（description，UID 无阅读意义）
    displayName: (owner, _repo, description) => {
      const d = (description ?? '').trim()
      return d || owner
    },
    searchQuery: (owner, _repo, description) => {
      const d = (description ?? '').trim()
      return d || owner
    },
    // 视频不生成 AI 摘要/翻译（标题简介均为中文，与后端 ai_eligible=false 对应）
    aiSummary: false,
    // tag 是 bvid，隐藏；显示 UP 主名
    showTag: false,
    // B 站视频封面布局（复用 YouTube 的封面 + 标题卡片）
    youtubeLayout: true,
  },
]

/** 按类型查注册表；未知类型返回 null（调用方回退默认行为）。 */
export function getSourceTypeDef(type: string): SourceTypeDef | undefined {
  return sourceTypeDefs.find(def => def.type === type)
}

/** 构造源标识键（source_type|owner|repo），避免不同类型同 owner/repo 串源。 */
export function sourceRepoKey(sourceType: string, owner: string, repo: string): string {
  return `${sourceType}|${owner}|${repo}`.toLowerCase()
}

/**
 * 统一解析输入：先按明确特征（域名 / @handle / 前缀）匹配，再按 fallback
 * 顺序兜底纯文本。新增类型只需在 sourceTypeDefs 登记 matches/parse。
 */
export function parseSourceUrl(raw: string): ParsedSource | null {
  const input = raw.trim()
  // 1) 明确特征优先（YouTube 链接/@handle/UCxxx、github.com、huggingface.co）
  for (const def of sourceTypeDefs) {
    if (def.matches?.(input)) {
      const r = def.parse(input)
      if (r) return { type: def.type, owner: r.owner, repo: r.repo }
    }
  }
  // 2) 无特征纯文本按 fallback 顺序兜底（github owner/repo 短格式 → HF 组织名）
  for (const def of sourceTypeDefs) {
    if (!def.fallback) continue
    const r = def.parse(input)
    if (r) return { type: def.type, owner: r.owner, repo: r.repo }
  }
  return null
}

/** Source 的展示辅助：显示名 / 搜索关键词，未覆盖类型回退 owner/repo。 */
export function sourceDisplayName(source: Source): string {
  const def = getSourceTypeDef(source.source_type)
  const name = def?.displayName?.(source.owner, source.repo, source.description)
  if (name) return name
  return source.repo ? `${source.owner}/${source.repo}` : source.owner
}

export function sourceSearchQuery(source: Source): string {
  const def = getSourceTypeDef(source.source_type)
  const q = def?.searchQuery?.(source.owner, source.repo, source.description)
  if (q) return q
  return `${source.owner}/${source.repo}`
}
