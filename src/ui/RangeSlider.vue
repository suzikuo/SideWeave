<script setup lang="ts">
/** 滑块（带值气泡）：拖动/悬停/聚焦时在拇指上方浮出当前值微卡 */
import { computed, ref, watch } from 'vue'

const props = withDefaults(
  defineProps<{
    modelValue: number
    min: number
    max: number
    step?: number
    /** 气泡文本格式化（默认直接显示数值） */
    format?: (v: number) => string
    ariaLabel?: string
  }>(),
  { step: 1 }
)

const emit = defineEmits<{ (e: 'commit', value: number): void }>()

/** 拖动过程中的本地草稿：气泡实时跟手；提交仍走 change，避免每次 input 都打 IPC */
const draft = ref(props.modelValue)
watch(
  () => props.modelValue,
  (v) => {
    draft.value = v
  }
)

const percent = computed(() => {
  const span = props.max - props.min
  if (span <= 0) return 0
  return Math.min(100, Math.max(0, ((draft.value - props.min) / span) * 100))
})

const bubbleText = computed(() =>
  props.format ? props.format(draft.value) : String(draft.value)
)

/** 拇指中心位置：线性百分比 + 拇指宽度（16px）补偿 */
const thumbLeft = computed(
  () => `calc(${percent.value}% + ${((0.5 - percent.value / 100) * 16).toFixed(1)}px)`
)

/** 两端时向内收一点，避免气泡溢出卡片 */
const bubbleShift = computed(() => {
  const p = percent.value
  const over = p < 20 ? (20 - p) / 20 : p > 80 ? (p - 80) / 20 : 0
  const px = over * 12 * (p < 20 ? 1 : -1)
  return `translateX(calc(-50% + ${px.toFixed(1)}px))`
})

function onInput(e: Event) {
  draft.value = Number((e.target as HTMLInputElement).value)
}
function onChange() {
  emit('commit', draft.value)
}
</script>

<template>
  <span class="rslider">
    <span class="bubble" :style="{ left: thumbLeft, transform: bubbleShift }">
      {{ bubbleText }}
    </span>
    <input
      type="range"
      :min="min"
      :max="max"
      :step="step"
      :value="draft"
      :aria-label="ariaLabel"
      :aria-valuetext="bubbleText"
      @input="onInput"
      @change="onChange"
    />
  </span>
</template>

<style scoped>
.rslider {
  position: relative;
  display: inline-flex;
  align-items: center;
}

.rslider input[type='range'] {
  width: 180px;
  accent-color: var(--accent);
  cursor: pointer;
}

.bubble {
  position: absolute;
  bottom: calc(100% + 8px);
  padding: 3px 8px;
  border-radius: var(--radius, 6px);
  background: var(--popover, var(--surface));
  border: 1px solid var(--border);
  color: var(--popover-foreground, var(--text));
  font-size: 11px;
  line-height: 1.5;
  white-space: nowrap;
  box-shadow: var(--shadow);
  pointer-events: none;
  opacity: 0;
  transition: opacity 0.16s ease;
  z-index: 5;
}

.rslider:hover .bubble,
.rslider:focus-within .bubble {
  opacity: 1;
}
</style>
