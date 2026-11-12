<script setup lang="ts">
import { reactive, watch } from 'vue'
import { FIELDS, OPERATORS, UNCATEGORIZED } from '../types'
import type { Rule } from '../types'
import { store } from '../stores/session'

const props = defineProps<{ rule: Rule }>()
const emit = defineEmits<{
  close: []
  save: [rule: Rule]
  test: [rule: Rule]
  newCategory: []
}>()

// Edit a detached copy so cancelling never mutates the live rule.
const draft = reactive<Rule>(JSON.parse(JSON.stringify(props.rule)) as Rule)
let seq = 0

// A category created while this dialog is open is assigned to the rule immediately.
watch(
  () => props.rule.categoryId,
  (id) => {
    draft.categoryId = id
  }
)

function plain(): Rule {
  return JSON.parse(JSON.stringify(draft)) as Rule
}

function addCondition(): void {
  draft.group.conditions.push({
    id: `c-${Date.now()}-${seq++}`,
    field: 'message',
    op: 'contains',
    value: '',
    caseSensitive: false,
    negate: false
  })
}

function removeCondition(id: string): void {
  draft.group.conditions = draft.group.conditions.filter((c) => c.id !== id)
}

function submit(): void {
  if (!draft.name.trim()) {
    store.error = '规则名称不能为空'
    return
  }
  const empty = draft.group.conditions.filter((c) => !c.value.trim())
  if (empty.length > 0) {
    store.error = '有条件的匹配值为空，请填写或删除'
    return
  }
  if (draft.group.conditions.length === 0) {
    store.error = '至少需要一条条件'
    return
  }
  emit('save', plain())
}
</script>

<template>
  <div class="modal-mask" @click.self="emit('close')">
    <div class="modal">
      <div class="modal-head">
        <span>{{ store.rules.some((r) => r.id === draft.id) ? '编辑规则' : '新建规则 / 模板另存为' }}</span>
        <div class="spacer" />
        <button class="btn btn-ghost btn-sm" @click="emit('close')">×</button>
      </div>

      <div class="modal-body">
        <div class="form-row">
          <span class="form-label">名称</span>
          <input v-model="draft.name" class="input" style="flex:1" placeholder="例如：支付流程异常" />
          <span class="form-label" style="width:48px">颜色</span>
          <input v-model="draft.color" class="input" style="width:110px" placeholder="#38bdf8" />
        </div>
        <div class="form-row">
          <span class="form-label">说明</span>
          <input v-model="draft.description" class="input" style="flex:1" placeholder="这条规则用来定位什么问题" />
        </div>
        <div class="form-row">
          <span class="form-label">所属分组</span>
          <select v-model="draft.categoryId" class="select" style="flex:1">
            <option :value="UNCATEGORIZED">未分组</option>
            <option v-for="c in store.categories" :key="c.id" :value="c.id">
              {{ c.name }}{{ c.builtin ? '（内置）' : '' }}
            </option>
          </select>
          <button class="btn btn-sm" title="新建一个分组" @click="emit('newCategory')">+ 分组</button>
        </div>
        <div class="form-row">
          <span class="form-label">条件组合</span>
          <label class="check"><input v-model="draft.group.logic" type="radio" value="AND" />全部满足 AND</label>
          <label class="check"><input v-model="draft.group.logic" type="radio" value="OR" />任一满足 OR</label>
          <div class="spacer" />
          <button class="btn btn-sm" @click="addCondition">+ 添加条件</button>
        </div>

        <div class="side-title" style="margin-top:14px">条件列表（{{ draft.group.conditions.length }}）</div>
        <div v-for="(c, i) in draft.group.conditions" :key="c.id" class="cond-row">
          <span class="muted mono" style="width:18px">{{ i + 1 }}</span>
          <select v-model="c.field" class="select" style="width:112px">
            <option v-for="f in FIELDS" :key="f.value" :value="f.value">{{ f.label }}</option>
          </select>
          <select v-model="c.op" class="select" style="width:126px">
            <option v-for="o in OPERATORS" :key="o.value" :value="o.value">{{ o.label }}</option>
          </select>
          <input v-model="c.value" class="input mono" placeholder="匹配值 / 正则表达式" />
          <div class="cond-extra">
            <label class="check" title="大小写敏感"><input v-model="c.caseSensitive" type="checkbox" />Aa</label>
            <label class="check" title="结果取反"><input v-model="c.negate" type="checkbox" />取反</label>
            <button class="btn btn-ghost btn-sm" title="删除条件" @click="removeCondition(c.id)">×</button>
          </div>
        </div>
        <p class="empty-hint" style="margin-top:10px">
          提示：正则使用 Rust regex 语法，默认不区分大小写；字段「原始行」包含堆栈等续行，适合做跨行匹配。
        </p>
        <div v-if="store.testCount !== null" class="test-result" style="margin-top:10px">
          最近一次测试命中：{{ store.testCount }} 条
        </div>
      </div>

      <div class="modal-foot">
        <button class="btn btn-sm" @click="emit('test', plain())">测试命中数</button>
        <div class="spacer" />
        <button class="btn btn-sm" @click="emit('close')">取消</button>
        <button class="btn btn-primary btn-sm" @click="submit">保存为自定义规则</button>
      </div>
    </div>
  </div>
</template>
