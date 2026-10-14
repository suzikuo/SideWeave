<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue'
import {
  getTimeClockStyle,
  timeCloseContextMenu,
  timeSetPinned,
  timeToggleClock,
  type ClockStyle
} from './api'

const style = ref<ClockStyle | null>(null)
const ready = ref(false)
let closing = false

async function closeMenu() {
  if (closing) return
  closing = true
  await timeCloseContextMenu().catch(() => {})
}

async function toggleLock() {
  if (!style.value) return
  const enabled = !style.value.pinned
  try {
    await timeSetPinned(enabled)
    style.value = { ...style.value, pinned: enabled }
  } finally {
    await closeMenu()
  }
}

async function hideClock() {
  try {
    await timeToggleClock()
  } finally {
    await closeMenu()
  }
}

function onKeydown(event: KeyboardEvent) {
  if (event.key === 'Escape') {
    event.preventDefault()
    void closeMenu()
  }
}

const onBlur = () => void closeMenu()

onMounted(async () => {
  window.addEventListener('blur', onBlur)
  window.addEventListener('keydown', onKeydown)
  try {
    style.value = await getTimeClockStyle()
  } finally {
    ready.value = true
  }
})

onUnmounted(() => {
  window.removeEventListener('blur', onBlur)
  window.removeEventListener('keydown', onKeydown)
})
</script>

<template>
  <main v-if="ready && style" class="menu" @contextmenu.prevent>
    <button class="menu-item" @click="toggleLock">
      {{ style.pinned ? '取消固定并恢复交互' : '固定并启用鼠标穿透' }}
    </button>
    <button class="menu-item danger" @click="hideClock">隐藏悬浮钟</button>
  </main>
</template>

<style>
:root,
html,
body,
#app {
  width: 100%;
  height: 100%;
  margin: 0;
  overflow: hidden;
  background: transparent;
}

* {
  box-sizing: border-box;
}

body {
  font-family: 'Segoe UI Variable', 'Segoe UI', 'Microsoft YaHei UI', sans-serif;
  color: #e8e9ed;
}

.menu {
  width: 100%;
  min-height: 100%;
  padding: 6px;
  border: 1px solid rgba(106, 108, 118, 0.8);
  border-radius: 12px;
  background: #1c1d22;
  box-shadow: 0 10px 28px rgba(0, 0, 0, 0.42);
  display: flex;
  flex-direction: column;
  gap: 2px;
  -webkit-app-region: no-drag;
}

.menu-item {
  width: 100%;
  min-height: 42px;
  padding: 8px 12px;
  border: 0;
  border-radius: 8px;
  background: transparent;
  color: #e8e9ed;
  font: inherit;
  font-size: 14px;
  line-height: 1.3;
  text-align: left;
  white-space: normal;
  overflow-wrap: anywhere;
  cursor: pointer;
  -webkit-app-region: no-drag;
}

.menu-item:hover {
  background: #2d3a61;
}

.menu-item.danger {
  color: #ffb4ab;
}

.menu-item.danger:hover {
  background: #4b292c;
}
</style>
