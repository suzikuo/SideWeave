<script setup lang="ts">
import { ref, watch } from 'vue'

const props = defineProps<{
  message: string
  kind?: 'info' | 'error' | 'success'
  duration?: number
}>()

const emit = defineEmits<{ (e: 'done'): void }>()

const visible = ref(false)
let timer: number | undefined

function show() {
  if (!props.message.trim()) {
    visible.value = false
    window.clearTimeout(timer)
    return
  }
  visible.value = true
  window.clearTimeout(timer)
  timer = window.setTimeout(() => {
    visible.value = false
    window.setTimeout(() => emit('done'), 300)
  }, props.duration ?? 3200)
}

watch(
  () => props.message,
  () => show(),
  { immediate: true }
)
</script>

<template>
  <Transition name="toast">
    <div v-if="visible" class="toast" :class="kind ?? 'info'">{{ message }}</div>
  </Transition>
</template>

<style scoped>
.toast {
  position: fixed;
  left: 50%;
  bottom: 28px;
  transform: translateX(-50%);
  max-width: 70%;
  padding: 9px 16px;
  border-radius: var(--radius, 12px);
  background: var(--surface-subtle);
  border: 1px solid var(--border);
  box-shadow: var(--shadow);
  font-size: 12.5px;
  color: var(--text);
  white-space: pre-wrap;
  word-break: break-all;
  z-index: 1000;
}
.toast.error {
  border-color: var(--danger);
  color: var(--danger);
}
.toast.success {
  border-color: var(--success);
  color: var(--success);
}
.toast-enter-active,
.toast-leave-active {
  transition: opacity 0.25s ease, transform 0.25s ease;
}
.toast-enter-from,
.toast-leave-to {
  opacity: 0;
  transform: translateX(-50%) translateY(8px);
}
</style>
