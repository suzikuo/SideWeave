<script setup lang="ts">
import { ref, watch } from 'vue'
import Icon from './Icon.vue'

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
    <div v-if="visible" class="toast" :class="kind ?? 'info'" role="status" aria-live="polite">
      <span class="toast-icon"><Icon :name="kind === 'error' ? 'x' : kind === 'success' ? 'check' : 'info'" :size="14" /></span>
      <span class="toast-message">{{ message }}</span>
    </div>
  </Transition>
</template>

<style scoped>
.toast {
  position: fixed;
  left: 50%;
  bottom: 28px;
  transform: translateX(-50%);
  max-width: 70%;
  display: flex;
  align-items: center;
  gap: 9px;
  padding: 9px 12px;
  border-radius: 8px;
  background: var(--popover, var(--surface));
  border: 1px solid var(--border);
  box-shadow: var(--shadow);
  font-size: 11px;
  color: var(--text);
  white-space: normal;
  z-index: 1000;
}
.toast-icon { display: grid; width: 22px; height: 22px; flex: none; place-items: center; border: 1px solid var(--border); border-radius: 6px; color: var(--text-muted); }
.toast-message { overflow-wrap: anywhere; }
.toast.error {
  border-color: color-mix(in srgb, var(--danger) 36%, var(--border));
  color: var(--text);
}
.toast.error .toast-icon { border-color: color-mix(in srgb, var(--danger) 30%, var(--border)); color: var(--danger); }
.toast.success {
  border-color: color-mix(in srgb, var(--success) 36%, var(--border));
  color: var(--text);
}
.toast.success .toast-icon { border-color: color-mix(in srgb, var(--success) 30%, var(--border)); color: var(--success); }
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
