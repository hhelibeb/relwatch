<script setup lang="ts">
import { computed, ref } from 'vue'
import ContextMenu from './common/ContextMenu.vue'
import type { ReleaseInfo } from '../api/releases'
import type { RepoGroup } from './releaseTypes'
import { useContextMenu } from '../composables/useContextMenu'
import { t } from '../i18n'
import { formatDate } from '../utils'
import { openReleaseUrl } from '../api/client'
import ReleaseItem from './ReleaseItem.vue'

const props = defineProps<{
  releases: ReleaseInfo[]
  isFiltering: boolean
}>()

const emit = defineEmits<{ update: [] }>()

const expandedRepos = ref<Set<string>>(new Set())

const repoGroups = computed(() => {
  const map = new Map<string, ReleaseInfo[]>()
  for (const release of props.releases) {
    const key = `${release.owner}/${release.repo}`
    if (!map.has(key)) map.set(key, [])
    map.get(key)!.push(release)
  }

  const groups: RepoGroup[] = []
  for (const [key, releases] of map) {
    releases.sort((a, b) => new Date(b.published_at).getTime() - new Date(a.published_at).getTime())
    groups.push({ key, releases })
  }

  groups.sort((a, b) => new Date(b.releases[0].published_at).getTime() - new Date(a.releases[0].published_at).getTime())
  return groups
})

const allExpanded = computed(() => {
  if (repoGroups.value.length === 0) return false
  return repoGroups.value.every(group => expandedRepos.value.has(group.key))
})

function toggleRepo(key: string) {
  const next = new Set(expandedRepos.value)
  if (next.has(key)) next.delete(key)
  else next.add(key)
  expandedRepos.value = next
}

function expandAll() {
  expandedRepos.value = new Set(repoGroups.value.map(group => group.key))
}

function toggleAllRepos() {
  if (allExpanded.value) {
    expandedRepos.value = new Set()
  } else {
    expandAll()
  }
}

defineExpose({ expandAll })

const {
  contextMenu: repoContextMenu,
  closeContextMenu: closeRepoContextMenu,
  handleContextMenu: handleRepoContextMenu,
  handleCopyLink: handleRepoCopyLink,
  handleOpenLink: handleRepoOpenLink,
} = useContextMenu()

function handleOpenUrl(url: string) {
  openReleaseUrl(url)
}
</script>

<template>
  <div v-if="repoGroups.length === 0" class="empty">
    {{ props.isFiltering ? t('release.no_match') : t('release.empty') }}
  </div>
  <div v-else class="repo-toolbar">
    <button class="btn-sm" @click="toggleAllRepos">
      <svg class="toggle-all-icon" :class="{ 'icon-collapse': allExpanded }"><use href="/icons.svg#chevron-down-icon"/></svg>
      {{ allExpanded ? t('release.collapse_all') : t('release.expand_all') }}
    </button>
  </div>
  <div v-for="group in repoGroups" :key="group.key" class="repo-group">
    <div class="repo-group-header" @click="toggleRepo(group.key)">
      <button class="repo-group-toggle" :class="{ expanded: expandedRepos.has(group.key) }" @click.stop="toggleRepo(group.key)">
        <svg><use href="/icons.svg#chevron-down-icon"/></svg>
      </button>
      <span class="repo-name">{{ group.key }}</span>
      <span class="repo-latest-tag">{{ group.releases[0].tag_name }}</span>
      <button class="btn-icon-link" @click.stop="handleOpenUrl(group.releases[0].html_url)" @contextmenu.prevent.stop="handleRepoContextMenu($event, group.releases[0].html_url)" :title="t('release.open_link')">
        <svg><use href="/icons.svg#link-icon"/></svg>
      </button>
      <span class="repo-latest-date">{{ formatDate(group.releases[0].published_at) }}</span>
      <span class="repo-meta">{{ t('release.versions', String(group.releases.length)) }}</span>
    </div>
    <div v-if="expandedRepos.has(group.key)" class="repo-group-body">
      <ReleaseItem
        v-for="release in group.releases"
        :key="release.id"
        :release="release"
        @update="emit('update')"
      />
    </div>
  </div>

  <ContextMenu v-if="repoContextMenu" :x="repoContextMenu.x" :y="repoContextMenu.y" @open="handleRepoOpenLink" @copy="handleRepoCopyLink" @close="closeRepoContextMenu" />
</template>

<style scoped>
.repo-group {
  background: var(--surface);
  border-radius: var(--radius);
  border: 1px solid var(--border);
  margin-bottom: 8px;
  overflow: hidden;
}

.repo-group-header {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 14px;
  cursor: pointer;
  user-select: none;
  transition: background 0.1s;
}

.repo-group-header:hover {
  background: var(--bg);
}

.repo-group-header .repo-name {
  font-weight: 600;
  font-size: 14px;
  flex: 1;
}

.repo-group-header .repo-latest-tag {
  font-weight: 600;
  font-size: 13px;
  color: var(--primary);
}

.repo-group-header .repo-latest-date {
  font-size: 12px;
  color: var(--text-muted);
  white-space: nowrap;
}

.repo-group-header .repo-meta {
  font-size: 12px;
  color: var(--text-muted);
  white-space: nowrap;
}

.repo-group-toggle {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  padding: 0;
  border: none;
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
  transition: transform 0.2s;
}

.repo-group-toggle svg {
  width: 14px;
  height: 14px;
}

.repo-group-toggle.expanded {
  transform: rotate(180deg);
}

.repo-toolbar {
  display: flex;
  justify-content: flex-end;
  margin-bottom: 6px;
}

.repo-toolbar .btn-sm {
  display: inline-flex;
  align-items: center;
  gap: 4px;
}

.toggle-all-icon {
  width: 12px;
  height: 12px;
  transition: transform 0.2s;
}

.icon-collapse {
  transform: rotate(180deg);
}

.repo-group-body {
  border-top: 1px solid var(--border);
  padding: 6px 14px 10px;
}

.repo-group-body .release-item {
  border: none;
  border-left: 4px solid var(--primary);
  margin-bottom: 6px;
  padding: 9px 12px;
  background: var(--bg);
}

.repo-group-body .release-item.release-importance-high {
  border-left-color: var(--danger);
}

.repo-group-body .release-item.release-importance-medium {
  border-left-color: #eab308;
}

.repo-group-body .release-item.release-importance-low {
  border-left-color: var(--success);
}

.repo-group-body .release-item:last-child {
  margin-bottom: 0;
}
</style>
