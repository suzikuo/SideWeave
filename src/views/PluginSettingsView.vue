<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { resetPluginSettings, setPluginSettings } from '../lib/api'
import { settingsRef } from '../lib/store'
import type { PluginMeta, PluginSettingField } from '../lib/types'
import Icon from '../ui/Icon.vue'
import Toast from '../ui/Toast.vue'

const props = defineProps<{ plugins: PluginMeta[] }>()
const selectedPluginId = ref('')
const selectionTouched = ref(false)
const draft = ref<Record<string, unknown>>({})
const busy = ref(false)
const toast = ref('')
const toastKind = ref<'info' | 'error' | 'success'>('info')

const selectedPlugin = computed(
  () => props.plugins.find((plugin) => plugin.id === selectedPluginId.value) ?? null
)
const fields = computed(() => selectedPlugin.value?.settings_schema ?? [])
const configured = computed(() =>
  Boolean(settingsRef.value?.plugins.configurations?.[selectedPluginId.value])
)

function showToast(message: string, kind: 'info' | 'error' | 'success' = 'info') {
  toastKind.value = kind
  toast.value = ''
  requestAnimationFrame(() => {
    toast.value = message
  })
}

function loadDraft() {
  const plugin = selectedPlugin.value
  if (!plugin) {
    draft.value = {}
    return
  }
  const saved = settingsRef.value?.plugins.configurations?.[plugin.id] ?? {}
  draft.value = Object.fromEntries(
    (plugin.settings_schema ?? []).map((field) => [
      field.key,
      Object.hasOwn(saved, field.key) ? saved[field.key] : field.default
    ])
  )
}

watch(selectedPluginId, loadDraft)
watch(
  () => props.plugins,
  () => {
    if (!selectionTouched.value || !props.plugins.some((plugin) => plugin.id === selectedPluginId.value)) {
      selectedPluginId.value = props.plugins.find((plugin) => plugin.settings_schema?.length)?.id
        ?? props.plugins[0]?.id
        ?? ''
    }
    loadDraft()
  },
  { deep: true, immediate: true }
)
watch(
  () => settingsRef.value?.plugins.configurations,
  loadDraft,
  { deep: true }
)

function update(field: PluginSettingField, value: unknown) {
  draft.value = { ...draft.value, [field.key]: value }
}

function updateNumber(field: PluginSettingField, event: Event) {
  const input = event.target as HTMLInputElement
  if (input.value === '') return
  const value = Number(input.value)
  if (Number.isFinite(value)) update(field, field.type === 'integer' ? Math.round(value) : value)
}

function updateText(field: PluginSettingField, event: Event) {
  update(field, (event.target as HTMLInputElement).value)
}

async function save() {
  const plugin = selectedPlugin.value
  if (!plugin || !fields.value.length || busy.value) return
  busy.value = true
  try {
    settingsRef.value = await setPluginSettings(plugin.id, { ...draft.value })
    showToast(`已保存 ${plugin.name} 的参数`, 'success')
  } catch (error) {
    showToast(String(error), 'error')
  } finally {
    busy.value = false
  }
}

async function restoreDefaults() {
  const plugin = selectedPlugin.value
  if (!plugin || !fields.value.length || busy.value) return
  busy.value = true
  try {
    settingsRef.value = await resetPluginSettings(plugin.id)
    loadDraft()
    showToast(`已恢复 ${plugin.name} 的插件默认值`, 'success')
  } catch (error) {
    showToast(String(error), 'error')
  } finally {
    busy.value = false
  }
}

</script>

<template>
  <div class="plugin-settings">
    <div class="settings-intro">
      <div>
        <h3>插件默认参数</h3>
        <p>表单由插件 manifest 的 settingsSchema 声明。保存值由宿主管理，插件可通过 Host API 读取。</p>
      </div>
      <label class="plugin-picker">
        <span>选择插件</span>
        <select v-model="selectedPluginId" @change="selectionTouched = true">
          <option v-for="plugin in plugins" :key="plugin.id" :value="plugin.id">
            {{ plugin.name }}
          </option>
        </select>
      </label>
    </div>

    <div v-if="!selectedPlugin" class="empty-state">
      <strong>没有可配置的插件</strong>
      <span>安装插件后，可以在这里编辑它声明的参数。</span>
    </div>

    <template v-else-if="fields.length">
      <div class="field-list">
        <div v-for="field in fields" :key="field.key" class="card row-card setting-field">
          <div class="row-main">
            <div class="row-title">{{ field.label }}</div>
            <div v-if="field.description" class="row-desc">{{ field.description }}</div>
          </div>

          <input
            v-if="field.type === 'text'"
            class="text-input"
            type="text"
            :value="String(draft[field.key] ?? '')"
            :placeholder="field.placeholder ?? ''"
            :maxlength="field.maxLength ?? 4096"
            :aria-label="`${selectedPlugin.name} ${field.label}`"
            @input="updateText(field, $event)"
          />
          <input
            v-else-if="field.type === 'number' || field.type === 'integer'"
            class="number-input"
            type="number"
            :value="Number(draft[field.key] ?? field.default)"
            :min="field.min ?? undefined"
            :max="field.max ?? undefined"
            :step="field.step ?? (field.type === 'integer' ? 1 : 'any')"
            :aria-label="`${selectedPlugin.name} ${field.label}`"
            @change="updateNumber(field, $event)"
          />
          <input
            v-else-if="field.type === 'boolean'"
            class="switch"
            type="checkbox"
            role="switch"
            :checked="Boolean(draft[field.key])"
            :aria-label="`${selectedPlugin.name} ${field.label}`"
            @change="update(field, ($event.target as HTMLInputElement).checked)"
          />
          <select
            v-else-if="field.type === 'select'"
            class="select-input"
            :value="String(draft[field.key] ?? field.default)"
            :aria-label="`${selectedPlugin.name} ${field.label}`"
            @change="update(field, ($event.target as HTMLSelectElement).value)"
          >
            <option v-for="option in field.options ?? []" :key="option.value" :value="option.value">
              {{ option.label }}
            </option>
          </select>
        </div>
      </div>

      <div class="settings-actions">
        <button class="btn" :disabled="busy || !configured" @click="restoreDefaults">
          <Icon name="refresh" :size="14" /> 恢复插件默认值
        </button>
        <button class="btn accent" :disabled="busy" @click="save">
          <Icon name="check" :size="14" /> {{ busy ? '保存中…' : '保存参数' }}
        </button>
      </div>
    </template>

    <div v-else class="empty-state">
      <strong>{{ selectedPlugin.name }} 尚未声明参数表单</strong>
      <span>插件作者可以在 manifest 的 settingsSchema 中声明字段，宿主会自动生成表单并校验输入。</span>
    </div>

    <Toast :message="toast" :kind="toastKind" @done="toast = ''" />
  </div>
</template>

<style scoped>
.plugin-settings {
  display: flex;
  flex-direction: column;
  gap: 14px;
  min-height: 0;
}

.settings-intro {
  display: flex;
  align-items: flex-end;
  justify-content: space-between;
  gap: 16px;
  padding: 3px 2px 7px;
}

.settings-intro h3 {
  margin: 0;
  color: var(--text);
  font-size: 14px;
  font-weight: 650;
}

.settings-intro p {
  max-width: 560px;
  margin: 6px 0 0;
  color: var(--text-muted);
  font-size: 11.5px;
  line-height: 1.6;
}

.plugin-picker {
  display: grid;
  flex: none;
  gap: 5px;
  min-width: 200px;
  color: var(--text-muted);
  font-size: 11px;
}

.plugin-picker select,
.select-input,
.text-input,
.number-input {
  min-height: 34px;
  padding: 0 10px;
  border: 1px solid var(--border);
  border-radius: 9px;
  outline: none;
  background: var(--surface);
  color: var(--text);
  font: inherit;
}

.plugin-picker select:focus-visible,
.select-input:focus-visible,
.text-input:focus-visible,
.number-input:focus-visible {
  border-color: var(--accent);
}

.field-list {
  display: grid;
  gap: 8px;
}

.setting-field {
  gap: 14px;
}

.text-input,
.number-input,
.select-input {
  flex: none;
  width: min(260px, 42%);
}

.empty-state {
  display: grid;
  gap: 7px;
  padding: 22px;
  border: 1px dashed var(--border);
  border-radius: 12px;
  color: var(--text-muted);
  font-size: 12px;
}

.empty-state strong {
  color: var(--text);
  font-size: 13px;
}

.settings-actions {
  display: flex;
  flex-wrap: wrap;
  justify-content: flex-end;
  gap: 8px;
  padding: 8px 0 4px;
}

@media (max-width: 680px) {
  .settings-intro {
    align-items: stretch;
    flex-direction: column;
  }

  .plugin-picker {
    width: 100%;
  }

  .text-input,
  .number-input,
  .select-input {
    width: 45%;
  }
}
</style>
