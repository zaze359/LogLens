<script setup lang="ts">
import { reactive } from 'vue'
import type { RuleCategory } from '../types'
import { saveCategory, store } from '../stores/session'

const props = defineProps<{ category: RuleCategory }>()
const emit = defineEmits<{
  close: []
  saved: [id: string]
}>()

// Detached copy: cancelling never mutates the live category.
const draft = reactive<RuleCategory>(JSON.parse(JSON.stringify(props.category)) as RuleCategory)

async function submit(): Promise<void> {
  if (!draft.name.trim()) {
    store.error = '分组名称不能为空'
    return
  }
  const id = await saveCategory(JSON.parse(JSON.stringify(draft)) as RuleCategory)
  if (id !== null) emit('saved', id)
}
</script>

<template>
  <div class="modal-mask" @click.self="emit('close')">
    <div class="modal modal-sm">
      <div class="modal-head">
        <span>{{ draft.id ? '编辑分组' : '新建分组' }}</span>
        <div class="spacer" />
        <button class="btn btn-ghost btn-sm" @click="emit('close')">×</button>
      </div>

      <div class="modal-body">
        <div class="form-row">
          <span class="form-label">名称</span>
          <input v-model="draft.name" class="input" style="flex:1" placeholder="例如：星学伴学生端 / 登录模块" />
          <span class="form-label" style="width:48px">颜色</span>
          <input v-model="draft.color" class="input" style="width:110px" placeholder="#34d399" />
        </div>
        <div class="form-row">
          <span class="form-label">说明</span>
          <input v-model="draft.description" class="input" style="flex:1" placeholder="这组规则用来排查什么问题" />
        </div>
        <p class="empty-hint">
          分组只做分类，不绑定具体应用：可以按应用、业务模块或问题类型自由划分。
          删除分组不会删除组内规则，规则会回到「未分组」。内置分组只读。
        </p>
        <p v-if="draft.builtin" class="empty-hint" style="color:var(--warning)">
          内置分组不可修改。
        </p>
      </div>

      <div class="modal-foot">
        <div class="spacer" />
        <button class="btn btn-sm" @click="emit('close')">取消</button>
        <button class="btn btn-primary btn-sm" :disabled="draft.builtin" @click="submit">保存分组</button>
      </div>
    </div>
  </div>
</template>
