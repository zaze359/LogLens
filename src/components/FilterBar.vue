<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { LEVELS, LEVEL_LABELS } from '../types'
import {
  clearRule,
  groupSummary,
  nextHit,
  prevHit,
  setSearchMode,
  store,
  updateFilter,
  updateSearch
} from '../stores/session'

const search = ref(store.filter.search)
const exclude = ref(store.filter.exclude)
const timeFrom = ref(store.filter.timeFrom)
const timeTo = ref(store.filter.timeTo)
const tagChoice = ref('')
const pidChoice = ref('')
const packageChoice = ref('')

/** `筛选` 会改变结果集，`查找` 只高亮定位 —— 两种模式共用一个关键字。 */
const isFind = computed(() => store.filter.searchMode === 'find')
const canJump = computed(() => (store.searchHits?.indices.length ?? 0) > 0)

const hitLabel = computed(() => {
  if (!store.filter.search) return '输入关键字'
  const hits = store.searchHits
  if (!hits) return store.searchError ? '查找失败' : '查找中…'
  if (hits.total === 0) return '无匹配'
  // 光标为空（比如原来是第 5 条，换筛选后那条没了）：只报总数，不假装停在第一条。
  if (store.searchCursor < 0) return `${hits.total} 处`
  const more = hits.truncated ? '+' : ''
  return `${store.searchCursor + 1} / ${hits.total}${more}`
})

function onEnter(e: KeyboardEvent): void {
  if (!isFind.value) return
  e.preventDefault()
  if (e.shiftKey) prevHit()
  else nextHit()
}

// Keep local inputs in sync when a filter is reset or a rule changes the spec.
watch(() => store.filter.search, (v) => { if (v !== search.value) search.value = v })
watch(() => store.filter.exclude, (v) => { if (v !== exclude.value) exclude.value = v })
watch(() => store.filter.timeFrom, (v) => { if (v !== timeFrom.value) timeFrom.value = v })
watch(() => store.filter.timeTo, (v) => { if (v !== timeTo.value) timeTo.value = v })

// 搜索框走 `updateSearch`：筛选模式下收窄列表，查找模式下只定位。
watch(search, (v) => updateSearch(v))
watch(exclude, (v) => updateFilter({ exclude: v }, true))
watch(timeFrom, (v) => updateFilter({ timeFrom: v }, true))
watch(timeTo, (v) => updateFilter({ timeTo: v }, true))

function toggleLevel(level: string): void {
  const next = store.filter.levels.includes(level)
    ? store.filter.levels.filter((l) => l !== level)
    : [...store.filter.levels, level]
  updateFilter({ levels: next })
}

function onTagChange(e: Event): void {
  const tag = (e.target as HTMLSelectElement).value
  tagChoice.value = ''
  if (!tag || store.filter.tags.includes(tag)) return
  updateFilter({ tags: [...store.filter.tags, tag] })
}

function onPackageChange(e: Event): void {
  const pkg = (e.target as HTMLSelectElement).value
  packageChoice.value = ''
  if (!pkg || store.filter.packages.includes(pkg)) return
  updateFilter({ packages: [...store.filter.packages, pkg] })
}

function removePackage(pkg: string): void {
  updateFilter({ packages: store.filter.packages.filter((p) => p !== pkg) })
}

function onPidChange(e: Event): void {
  const pid = (e.target as HTMLSelectElement).value
  pidChoice.value = ''
  if (!pid || store.filter.pids.includes(pid)) return
  updateFilter({ pids: [...store.filter.pids, pid] })
}

function removeTag(tag: string): void {
  updateFilter({ tags: store.filter.tags.filter((t) => t !== tag) })
}

function removePid(pid: string): void {
  updateFilter({ pids: store.filter.pids.filter((p) => p !== pid) })
}

function onRegex(e: Event): void {
  updateFilter({ isRegex: (e.target as HTMLInputElement).checked })
}

function onCaseSensitive(e: Event): void {
  updateFilter({ caseSensitive: (e.target as HTMLInputElement).checked })
}

function onGroupLogic(e: Event): void {
  updateFilter({ groupLogic: (e.target as HTMLSelectElement).value })
}

function toggleOrder(): void {
  updateFilter({ order: store.filter.order === 'asc' ? 'desc' : 'asc' })
}
</script>

<template>
  <div class="filterbar">
    <div class="filter-row">
      <span class="filter-label">级别</span>
      <button
        v-for="level in LEVELS"
        :key="level"
        class="chip level-chip"
        :class="{ on: store.filter.levels.includes(level) }"
        :title="LEVEL_LABELS[level]"
        @click="toggleLevel(level)"
      >{{ level }}</button>

      <div class="divider-v" />
      <span class="filter-label">包名</span>
      <select class="select" style="width: 186px" :value="packageChoice" @change="onPackageChange">
        <option value="">+ 添加包名</option>
        <option v-for="pkg in store.fileInfo?.packages ?? []" :key="pkg" :value="pkg">{{ pkg }}</option>
      </select>
      <span v-for="pkg in store.filter.packages" :key="pkg" class="chip on" :title="pkg">
        {{ pkg }} <button class="btn btn-ghost btn-sm x" style="height:16px;padding:0 3px" @click="removePackage(pkg)">×</button>
      </span>

      <div class="divider-v" />
      <span class="filter-label">Tag</span>
      <select class="select" style="width: 132px" :value="tagChoice" @change="onTagChange">
        <option value="">+ 添加 Tag</option>
        <option v-for="tag in store.fileInfo?.tags ?? []" :key="tag" :value="tag">{{ tag }}</option>
      </select>
      <span v-for="tag in store.filter.tags" :key="tag" class="chip on">
        {{ tag }} <button class="btn btn-ghost btn-sm x" style="height:16px;padding:0 3px" @click="removeTag(tag)">×</button>
      </span>

      <span class="filter-label" style="margin-left:4px">PID</span>
      <select class="select" style="width: 108px" :value="pidChoice" @change="onPidChange">
        <option value="">+ 添加 PID</option>
        <option v-for="pid in store.fileInfo?.pids ?? []" :key="pid" :value="pid">{{ pid }}</option>
      </select>
      <span v-for="pid in store.filter.pids" :key="pid" class="chip on">
        {{ pid }} <button class="btn btn-ghost btn-sm x" style="height:16px;padding:0 3px" @click="removePid(pid)">×</button>
      </span>
    </div>

    <div class="filter-row">
      <div class="seg" role="group" aria-label="搜索模式">
        <button
          class="seg-btn"
          :class="{ on: !isFind }"
          title="筛选：只显示匹配的行，命中数会变"
          @click="setSearchMode('filter')"
        >筛选</button>
        <button
          class="seg-btn"
          :class="{ on: isFind }"
          title="查找：只高亮并跳转，不改变列表"
          @click="setSearchMode('find')"
        >查找</button>
      </div>
      <input
        v-model="search"
        class="input search-input mono"
        :class="{ 'search-input-compact': isFind }"
        :placeholder="
          isFind
            ? '在结果中查找（高亮 + 跳转，不改列表）'
            : '关键字 / 正则，匹配整行（含堆栈）'
        "
        @keydown.enter="onEnter"
      />
      <template v-if="isFind">
        <span class="hit-count" :class="{ empty: !canJump }">{{ hitLabel }}</span>
        <button class="btn btn-sm" :disabled="!canJump" title="上一个（Shift+Enter）" @click="prevHit">↑</button>
        <button class="btn btn-sm" :disabled="!canJump" title="下一个（Enter）" @click="nextHit">↓</button>
        <span v-if="store.searchError" class="hit-error">{{ store.searchError }}</span>
      </template>
      <label class="check"><input type="checkbox" :checked="store.filter.isRegex" @change="onRegex" />正则</label>
      <label class="check"><input type="checkbox" :checked="store.filter.caseSensitive" @change="onCaseSensitive" />区分大小写</label>

      <div class="divider-v" />
      <span class="filter-label">排除</span>
      <input v-model="exclude" class="input mini-input" placeholder="逗号分隔" />

      <div class="divider-v" />
      <span class="filter-label">时间</span>
      <input v-model="timeFrom" class="input mini-input mono" placeholder="HH:MM:SS" />
      <span class="muted">~</span>
      <input v-model="timeTo" class="input mini-input mono" placeholder="HH:MM:SS" />

      <div class="divider-v" />
      <button class="btn btn-sm" @click="toggleOrder">
        {{ store.filter.order === 'asc' ? '↑ 正序' : '↓ 倒序' }}
      </button>
    </div>

    <div class="filter-row">
      <template v-if="store.filter.group">
        <span class="active-rule">规则已应用 · {{ groupSummary(store.filter.group) }}</span>
        <span class="filter-label">与快捷筛选</span>
        <select class="select" :value="store.filter.groupLogic" @change="onGroupLogic">
          <option value="AND">AND（取交集）</option>
          <option value="OR">OR（取并集）</option>
        </select>
        <button class="btn btn-ghost btn-sm" @click="clearRule">移除规则</button>
      </template>
      <span v-else class="muted">未应用规则：从左侧选择内置模板或自定义规则，也可只使用上方快捷筛选</span>
    </div>
  </div>
</template>
