<script setup lang="ts">
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { EVENTS, getMonitorOptions } from '../lib/api'
import type { MonitorOption } from '../lib/types'

const props = defineProps<{
  modelValue: string | null
}>()

const emit = defineEmits<{
  'update:modelValue': [value: string | null]
}>()

const options = ref<MonitorOption[]>([])
let unlisten: UnlistenFn | undefined
let disposed = false
const savedTargetUnavailable = computed(
  () => !!props.modelValue && !options.value.some((option) => option.id === props.modelValue)
)

async function refreshOptions() {
  try {
    options.value = await getMonitorOptions()
  } catch {
    options.value = []
  }
}

onMounted(async () => {
  const stopListening = await listen(EVENTS.monitorsChanged, () => {
    void refreshOptions()
  }).catch(() => undefined)
  if (disposed) stopListening?.()
  else unlisten = stopListening
  await refreshOptions()
})

onUnmounted(() => {
  disposed = true
  unlisten?.()
})

function onChange(event: Event) {
  const value = (event.target as HTMLSelectElement).value
  emit('update:modelValue', value || null)
}
</script>

<template>
  <select
    class="monitor-picker"
    :value="modelValue ?? ''"
    aria-label="侧边栏显示器"
    @change="onChange"
  >
    <option value="">自动（跟随鼠标所在屏幕）</option>
    <option
      v-if="savedTargetUnavailable"
      :value="modelValue"
    >
      已保存的显示器暂不可用（临时使用主屏）
    </option>
    <option
      v-for="(option, index) in options"
      :key="option.id ?? `unavailable-${index}`"
      :value="option.id ?? `unavailable-${index}`"
      :disabled="!option.id"
    >
      {{ option.label }}
    </option>
  </select>
</template>

<style scoped>
.monitor-picker {
  width: min(280px, 48%);
  min-width: 175px;
  height: 34px;
  padding: 0 30px 0 10px;
  border: 1px solid var(--border);
  border-radius: 8px;
  background: var(--input);
  color: var(--foreground);
  font: inherit;
  font-size: 12px;
  outline: none;
}

.monitor-picker:focus-visible {
  border-color: var(--ring);
  box-shadow: 0 0 0 2px color-mix(in srgb, var(--ring) 20%, transparent);
}

@media (max-width: 560px) {
  .monitor-picker {
    width: 100%;
    min-width: 0;
  }
}
</style>
