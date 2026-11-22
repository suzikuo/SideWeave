<script setup lang="ts">
import { onMounted, onUnmounted } from 'vue'
import Button from '@/components/ui/button/Button.vue'
import Icon from './Icon.vue'

const props = withDefaults(
  defineProps<{
    open: boolean
    title: string
    message: string
    confirmText?: string
    cancelText?: string
    danger?: boolean
  }>(),
  {
    confirmText: '确认',
    cancelText: '取消',
    danger: false
  }
)

const emit = defineEmits<{
  (e: 'confirm'): void
  (e: 'cancel'): void
}>()

function onKeydown(event: KeyboardEvent) {
  if (!props.open) return
  if (event.key === 'Escape') {
    event.preventDefault()
    event.stopPropagation()
    emit('cancel')
  }
}

onMounted(() => window.addEventListener('keydown', onKeydown, true))
onUnmounted(() => window.removeEventListener('keydown', onKeydown, true))
</script>

<template>
  <Teleport to="body">
    <Transition name="confirm-fade">
      <div
        v-if="open"
        class="confirm-backdrop"
        role="presentation"
        @click.self="emit('cancel')"
      >
        <div
          class="confirm-dialog"
          role="alertdialog"
          aria-modal="true"
          :aria-label="title"
        >
          <header class="confirm-header">
            <span class="confirm-icon" :class="{ danger }">
              <Icon :name="danger ? 'info' : 'check'" :size="16" />
            </span>
            <h3>{{ title }}</h3>
          </header>
          <p class="confirm-body">{{ message }}</p>
          <footer class="confirm-actions">
            <Button variant="outline" size="sm" type="button" @click="emit('cancel')">
              {{ cancelText }}
            </Button>
            <Button
              :variant="danger ? 'destructive' : 'default'"
              size="sm"
              type="button"
              @click="emit('confirm')"
            >
              {{ confirmText }}
            </Button>
          </footer>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.confirm-backdrop {
  position: fixed;
  inset: 0;
  z-index: 2000;
  display: grid;
  place-items: center;
  padding: 20px;
  background: rgb(0 0 0 / 52%);
  backdrop-filter: blur(3px);
}
.confirm-dialog {
  width: min(100%, 440px);
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 18px 20px;
  border: 1px solid var(--border);
  border-radius: 12px;
  background: var(--popover, var(--card, var(--surface)));
  color: var(--foreground, var(--text));
  box-shadow: 0 16px 40px rgb(0 0 0 / 32%);
}
.confirm-header {
  display: flex;
  align-items: center;
  gap: 10px;
}
.confirm-header h3 {
  margin: 0;
  font-size: 14px;
  font-weight: 600;
  line-height: 1.3;
}
.confirm-icon {
  display: grid;
  width: 28px;
  height: 28px;
  flex: none;
  place-items: center;
  border-radius: 8px;
  border: 1px solid color-mix(in srgb, var(--primary) 28%, var(--border));
  background: color-mix(in srgb, var(--primary) 10%, transparent);
  color: var(--primary);
}
.confirm-icon.danger {
  border-color: color-mix(in srgb, var(--destructive, var(--danger)) 32%, var(--border));
  background: color-mix(in srgb, var(--destructive, var(--danger)) 12%, transparent);
  color: var(--destructive, var(--danger));
}
.confirm-body {
  margin: 0;
  color: var(--muted-foreground, var(--text-muted));
  font-size: 12.5px;
  line-height: 1.6;
  white-space: pre-line;
  overflow-wrap: anywhere;
  max-height: 50vh;
  overflow-y: auto;
}
.confirm-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  padding-top: 4px;
}
.confirm-fade-enter-active,
.confirm-fade-leave-active {
  transition: opacity 0.16s ease;
}
.confirm-fade-enter-from,
.confirm-fade-leave-to {
  opacity: 0;
}
</style>
