<script setup lang="ts">
import { ref } from 'vue'
import { UNCATEGORIZED } from '../types'
import type { Rule, RuleCategory } from '../types'
import ConditionEditor from './ConditionEditor.vue'
import CategoryEditor from './CategoryEditor.vue'
import {
  applyRule,
  clearRule,
  deleteCategory,
  deleteRule,
  expandAll,
  exportRulesFile,
  groupSummary,
  importRulesFile,
  isCollapsed,
  newCategory,
  newRule,
  ruleFromTemplate,
  saveRule,
  searching,
  store,
  testRule,
  toggleCategory,
  totalRuleCount,
  visibleRuleCount,
  visibleSections
} from '../stores/session'
import type { RuleSection } from '../stores/session'

const editorRule = ref<Rule | null>(null)
const editorCategory = ref<RuleCategory | null>(null)
const confirmDeleteId = ref('')
let confirmTimer: ReturnType<typeof setTimeout> | null = null

function edit(rule: Rule): void {
  editorRule.value = JSON.parse(JSON.stringify(rule)) as Rule
}

function create(categoryId: string = UNCATEGORIZED): void {
  editorRule.value = newRule(categoryId)
}

function copyTemplate(rule: Rule): void {
  editorRule.value = ruleFromTemplate(rule)
}

async function onSave(rule: Rule): Promise<void> {
  await saveRule(rule)
  editorRule.value = null
}

function onTest(rule: Rule): void {
  void testRule(rule)
}

function createCategory(): void {
  editorCategory.value = newCategory()
}

function editCategory(section: RuleSection): void {
  editorCategory.value = {
    id: section.id,
    name: section.name,
    description: section.description,
    color: section.color,
    builtin: section.builtin
  }
}

/** A category created from inside the rule editor is selected immediately. */
function onCategorySaved(id: string): void {
  editorCategory.value = null
  if (editorRule.value && id) editorRule.value.categoryId = id
}

async function onDeleteCategory(id: string): Promise<void> {
  if (confirmTimer) {
    clearTimeout(confirmTimer)
    confirmTimer = null
  }
  if (confirmDeleteId.value !== id) {
    confirmDeleteId.value = id
    confirmTimer = setTimeout(() => {
      confirmDeleteId.value = ''
      confirmTimer = null
    }, 4000)
    return
  }
  confirmDeleteId.value = ''
  await deleteCategory(id)
}
</script>

<template>
  <aside class="sidebar">
    <div class="sidebar-tools">
      <div class="search-box">
        <span class="search-icon">⌕</span>
        <input
          v-model="store.ruleSearch"
          class="input"
          placeholder="搜索规则 / 分组 / 条件值"
        />
        <button v-if="store.ruleSearch" class="btn btn-ghost btn-sm" title="清空搜索" @click="store.ruleSearch = ''">×</button>
      </div>
      <div class="sidebar-tool-row">
        <button class="btn btn-sm" @click="create()">+ 规则</button>
        <button class="btn btn-sm" @click="createCategory()">+ 分组</button>
        <div class="spacer" />
        <button class="btn btn-ghost btn-sm" title="展开全部分组" @click="expandAll">展开</button>
      </div>
      <div class="search-stat">
        <template v-if="searching">命中 {{ visibleRuleCount }} / {{ totalRuleCount }} 条规则</template>
        <template v-else>{{ totalRuleCount }} 条规则 · {{ store.categories.filter((c) => !c.builtin).length }} 个自定义分组</template>
      </div>
    </div>

    <div class="sidebar-scroll">
      <section
        v-for="section in visibleSections"
        :key="section.id || '__uncategorised__'"
        class="cat-section"
      >
        <div
          class="cat-head"
          :title="section.description || section.name"
          @click="toggleCategory(section.id)"
        >
          <span class="cat-caret" :class="{ collapsed: isCollapsed(section.id) }">▾</span>
          <span class="cat-dot" :style="{ background: section.color || '#64748b' }" />
          <span class="cat-name">{{ section.name }}</span>
          <span v-if="section.builtin" class="badge">内置</span>
          <span class="cat-count mono">{{ searching ? `${section.rules.length}/${section.total}` : section.total }}</span>
          <div v-if="!section.builtin && section.id !== UNCATEGORIZED" class="cat-actions" @click.stop>
            <button class="btn btn-ghost btn-sm" title="在此分组新建规则" @click="create(section.id)">+</button>
            <button class="btn btn-ghost btn-sm" title="编辑分组" @click="editCategory(section)">改</button>
            <button
              class="btn btn-ghost btn-sm"
              :class="{ 'btn-confirm': confirmDeleteId === section.id }"
              :title="confirmDeleteId === section.id ? '再次点击确认删除' : '删除分组（规则会移到未分组）'"
              @click="onDeleteCategory(section.id)"
            >{{ confirmDeleteId === section.id ? '确认?' : '×' }}</button>
          </div>
        </div>

        <div v-if="!isCollapsed(section.id)" class="cat-rules">
          <div
            v-for="rule in section.rules"
            :key="rule.id"
            class="rule-item"
            :class="{ active: store.activeRuleId === rule.id }"
            :title="rule.description"
            @click="applyRule(rule)"
          >
            <span class="rule-dot" :style="{ background: rule.color || '#38bdf8' }" />
            <div class="rule-body">
              <div class="rule-name">
                {{ rule.name }}
                <span v-if="rule.builtin" class="inline-badge">模板</span>
              </div>
              <div class="rule-desc">{{ rule.description || '（无说明）' }}</div>
              <div class="rule-desc mono">{{ groupSummary(rule.group) }}</div>
            </div>
            <div class="rule-actions">
              <button v-if="rule.builtin" class="btn btn-ghost btn-sm" title="另存为可编辑的自定义规则" @click.stop="copyTemplate(rule)">另存</button>
              <template v-else>
                <button class="btn btn-ghost btn-sm" title="编辑" @click.stop="edit(rule)">改</button>
                <button class="btn btn-ghost btn-sm" title="删除" @click.stop="deleteRule(rule.id)">×</button>
              </template>
            </div>
          </div>
        </div>
      </section>

      <p v-if="visibleSections.length === 0" class="empty-hint">
        {{ searching ? '没有匹配的规则或分组，试试更短的关键词。' : '还没有规则，点击「+ 规则」开始。' }}
      </p>
      <button v-if="store.activeRuleId" class="btn btn-sm" style="width:100%;justify-content:center;margin-top:8px" @click="clearRule">
        清除当前规则
      </button>
    </div>

    <div class="sidebar-footer">
      <div style="display:flex;gap:6px">
        <button class="btn btn-sm" style="flex:1;justify-content:center" @click="importRulesFile">导入规则</button>
        <button class="btn btn-sm" style="flex:1;justify-content:center" @click="exportRulesFile">导出规则</button>
      </div>
      <div v-if="store.rulesPath" class="empty-hint mono" style="margin-top:8px;word-break:break-all" :title="store.rulesPath">
        配置：{{ store.rulesPath }}
      </div>
    </div>

    <ConditionEditor
      v-if="editorRule"
      :rule="editorRule"
      @close="editorRule = null"
      @save="onSave"
      @test="onTest"
      @new-category="createCategory"
    />
    <CategoryEditor
      v-if="editorCategory"
      :category="editorCategory"
      @close="editorCategory = null"
      @saved="onCategorySaved"
    />
  </aside>
</template>
