import type { ReleaseInfo } from '../api/releases'
import type { RepoGroup } from '../components/releaseTypes'
import { sourceRepoKey } from '../api/source-registry'

function timeOf(release: ReleaseInfo): number {
  return new Date(release.published_at).getTime()
}

/**
 * 按源仓库分组，返回**界面展示顺序**：组按组内最新版本发布时间降序，
 * 组内版本同样降序。
 *
 * 聚合列表与批量选择的「可见项」必须共用这一份顺序——选择区间、全选范围
 * 都要与用户眼前看到的行序一致。
 */
export function groupReleasesByRepo(releases: ReleaseInfo[]): RepoGroup[] {
  const map = new Map<string, ReleaseInfo[]>()
  for (const release of releases) {
    // 分组键含 source_type，避免不同类型同 owner/repo 串源
    const key = sourceRepoKey(release.source_type, release.owner, release.repo)
    if (!map.has(key)) map.set(key, [])
    map.get(key)!.push(release)
  }

  const groups: RepoGroup[] = []
  for (const [key, list] of map) {
    list.sort((a, b) => timeOf(b) - timeOf(a))
    groups.push({ key, releases: list })
  }
  groups.sort((a, b) => timeOf(b.releases[0]) - timeOf(a.releases[0]))
  return groups
}

/** 已展开分组内的版本，按界面展示顺序摊平（折叠分组的版本不在可见范围内）。 */
export function visibleGroupedReleases(
  releases: ReleaseInfo[],
  expanded: ReadonlySet<string>,
): ReleaseInfo[] {
  return groupReleasesByRepo(releases)
    .filter((group) => expanded.has(group.key))
    .flatMap((group) => group.releases)
}
