<script setup lang="ts">
import Button from '@/components/ui/button/Button.vue'
import Switch from '@/components/ui/switch/Switch.vue'
import { invoke } from '@tauri-apps/api/core'
/** 简单设置（#/settings）：单列快速设置流；插件管理、系统信息与诊断请前往控制中心 */
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import Icon from '../ui/Icon.vue'
import HotkeyRecorder from '../ui/HotkeyRecorder.vue'
import MonitorPicker from '../ui/MonitorPicker.vue'
import RangeSlider from '../ui/RangeSlider.vue'
import Toast from '../ui/Toast.vue'
import { getPerformanceStatus, setSetting } from '../lib/api'
import { marketInstalled } from '../lib/marketApi'
import { applySkinWithRecommendedOpacity, type InstalledSkinAppearance } from '../lib/skinAppearance'
import { autostartRef, settingsRef } from '../lib/store'
import type { PerformanceProfile, PerformanceStatusPayload } from '../lib/types'

const ACCENT_SWATCHES = ['#006874', '#006A6A', '#4F6354', '#7A5900', '#6750A4']

const settings = computed(() => settingsRef.value)
const userSkins = ref<InstalledSkinAppearance[]>([])
const activeSkinId = computed(() => settings.value?.appearance.skin ?? 'default')
const skinOptions = computed(() => [
  { id: 'default', label: '默认' },
  ...userSkins.value.map((skin) => ({ id: `user:${skin.id}`, label: skin.name }))
])

async function reloadUserSkins() {
  try {
    userSkins.value = (await marketInstalled()).skins.map(({ id, name, appearanceDefaults }) => ({
      id,
      name,
      appearanceDefaults
    }))
  } catch {
    // Keep the default option available if the installed skin list cannot be read.
  }
}

watch(activeSkinId, (skinId, previousSkinId) => {
  if (skinId !== previousSkinId && skinId.startsWith('user:')) void reloadUserSkins()
})

const perfStatus = ref<PerformanceStatusPayload | null>(null)

const profileLabels: Record<PerformanceProfile, string> = {
  eco: '省资源',
  balanced: '平衡',
  performance: '极速',
  auto: '自动'
}

async function loadPerfStatus() {
  try {
    perfStatus.value = await getPerformanceStatus()
  } catch (e) {
    console.error('获取性能状态失败', e)
  }
}

const perfDesc = computed(() => {
  if (!perfStatus.value) return '自适应调度策略'
  const { userProfile, effectiveProfile, memory } = perfStatus.value
  const effLabel = profileLabels[effectiveProfile] || effectiveProfile
  const memText = memory
    ? `（可用 ${(memory.availPhysBytes / (1024 ** 3)).toFixed(1)} GB / 负荷 ${memory.memoryLoadPct}%）`
    : ''
  if (userProfile === 'auto') {
    return `根据系统可用内存与负荷动态决策 · 当前生效：${effLabel}${memText}`
  }
  if (userProfile === 'performance') {
    return `窗口后台静默预热 · 双窗口常驻保活 · 极速 120ms 响应${memText}`
  }
  if (userProfile === 'balanced') {
    return `侧边栏与悬浮预览保活 · 详情卡闲置 30s 销毁回收${memText}`
  }
  return `按需创建 WebView · 关闭即刻销毁 · 极低资源占用${memText}`
})

async function changePerformanceProfile(profile: PerformanceProfile) {
  await change('general', 'performance_profile', profile)
  await loadPerfStatus()
}

const toast = ref('')
const toastKind = ref<'info' | 'error' | 'success'>('info')
function showToast(message: string, kind: 'info' | 'error' | 'success' = 'info') {
  toastKind.value = kind
  toast.value = ''
  requestAnimationFrame(() => {
    toast.value = message
  })
}

// ---------- 主题生效标记（本地 matchMedia，不写 lib/） ----------

const darkQuery = window.matchMedia('(prefers-color-scheme: dark)')
const systemDark = ref(darkQuery.matches)
function onSchemeChange(e: MediaQueryListEvent) {
  systemDark.value = e.matches
}
/** 实际生效主题：显式取所选，system 时跟随系统 */
const effectiveModeLabel = computed(() => {
  const mode = settingsRef.value?.appearance.theme_mode
  const eff = !mode || mode === 'system' ? (systemDark.value ? 'dark' : 'light') : mode
  return eff === 'dark' ? '深色' : '浅色'
})

/** 滑块气泡格式化 */
const fmtMs = (v: number) => `${v} ms`
const fmtPx = (v: number) => `${v} px`
const fmtPercent = (v: number) => `${Math.round(v * 100)}%`
const fmtRows = (v: number) => `${v} 条`

onMounted(() => {
  darkQuery.addEventListener('change', onSchemeChange)
  loadPerfStatus()
  void reloadUserSkins()
})
onUnmounted(() => darkQuery.removeEventListener('change', onSchemeChange))

async function change(category: string, key: string, value: unknown) {
  try {
    if (category === 'appearance' && key === 'skin') {
      const skinId = String(value)
      if (settingsRef.value?.appearance.skin === skinId) return
      settingsRef.value = await applySkinWithRecommendedOpacity(skinId, userSkins.value)
    } else {
      settingsRef.value = await setSetting(category, key, value)
    }
    if (category === 'general' && key === 'run_on_startup') {
      autostartRef.value = Boolean(value)
    }
  } catch (e) {
    showToast(String(e), 'error')
  }
}

/** 热键提交：Rust 校验并即时重注册；失败时 settingsRef 未变，自动回显旧值 */
async function changeHotkey(value: string) {
  try {
    settingsRef.value = await setSetting('shortcuts', 'toggle_sidebar', value)
    showToast('热键已更新', 'success')
  } catch (e) {
    showToast(String(e), 'error')
  }
}

/** 打开控制中心（插件管理、系统信息与诊断），与详情页复用同一个窗口 */
async function changeControlCenterHotkey(value: string) {
  try {
    settingsRef.value = await setSetting('shortcuts', 'control_center', value)
    showToast(value ? '热键已更新' : '热键已清除', 'success')
  } catch (e) {
    showToast(String(e), 'error')
  }
}

async function openControlCenter() {
  try {
    await invoke<void>('open_control_center')
  } catch (e) {
    showToast(String(e), 'error')
  }
}
</script>

<template>
  <div v-if="settings" class="settings">
    <header class="settings-hero">
      <div class="settings-hero-icon"><Icon name="settings" :size="17" /></div>
      <div><div class="settings-hero-kicker">PREFERENCES</div><h1>快捷设置</h1><p>调整 SideWeave 的外观、行为和全局快捷键。</p></div>
    </header>

    <section class="settings-group">
      <div class="settings-group-heading"><div><h2>外观</h2><p>让工作区适合你的屏幕和使用习惯。</p></div></div>
      <div class="settings-group-card">
    <div class="card row-card">
      <div class="row-main">
        <div class="row-title">界面缩放</div>
        <div class="row-desc">调整宿主窗口和插件页面的显示大小</div>
      </div>
      <RangeSlider
        :model-value="settings.appearance.interface_scale"
        :min="0.7"
        :max="1.6"
        :step="0.01"
        :format="fmtPercent"
        aria-label="界面缩放"
        @commit="(v) => change('appearance', 'interface_scale', v)"
      />
    </div>

    <!-- 主题 -->
    <div class="card row-card">
      <div class="row-main">
        <div class="row-title">主题</div>
        <div class="row-desc">当前生效：{{ effectiveModeLabel }}</div>
      </div>
      <div class="segmented">
        <button
          v-for="m in (['light', 'dark', 'system'] as const)"
          :key="m"
          :class="{ on: settings.appearance.theme_mode === m }"
          :aria-label="`主题：${m === 'light' ? '浅色' : m === 'dark' ? '深色' : '跟随系统'}`"
          @click="change('appearance', 'theme_mode', m)"
        >
          {{ m === 'light' ? '浅色' : m === 'dark' ? '深色' : '跟随系统' }}
        </button>
      </div>
    </div>

    <!-- 皮肤 -->
    <div class="card row-card">
      <div class="row-main">
        <div class="row-title">皮肤</div>
        <div class="row-desc">表现形式，与主题独立组合</div>
      </div>
      <div class="segmented">
        <button
          v-for="s in skinOptions"
          :key="s.id"
          :class="{ on: settings.appearance.skin === s.id }"
          :aria-label="`皮肤：${s.label}`"
          @click="change('appearance', 'skin', s.id)"
        >
          {{ s.label }}
        </button>
      </div>
    </div>

    <!-- 强调色 -->
    <div class="card row-card">
      <div class="row-main">
        <div class="row-title">强调色</div>
      </div>
      <div class="swatches">
        <button
          v-for="c in ACCENT_SWATCHES"
          :key="c"
          class="swatch"
          :style="{ background: c }"
          :class="{ on: settings.appearance.accent_color.toUpperCase() === c.toUpperCase() }"
          :aria-label="`强调色 ${c}`"
          @click="change('appearance', 'accent_color', c)"
        />
        <input
          type="color"
          class="swatch custom"
          :value="settings.appearance.accent_color"
          aria-label="自定义强调色"
          @change="change('appearance', 'accent_color', ($event.target as HTMLInputElement).value)"
        />
      </div>
    </div>
      </div>
    </section>

<!-- 停靠位置 -->
    <section class="settings-group">
      <div class="settings-group-heading"><div><h2>侧边栏行为</h2><p>设置停靠方式、展开逻辑和预览响应。</p></div></div>
      <div class="settings-group-card">
    <div class="card row-card">
      <div class="row-main">
        <div class="row-title">停靠位置</div>
      </div>
      <div class="segmented">
        <button
          v-for="p in (['left', 'right', 'top'] as const)"
          :key="p"
          :class="{ on: settings.appearance.sidebar_position === p }"
          :aria-label="`停靠位置：${p === 'left' ? '左侧' : p === 'right' ? '右侧' : '顶部'}`"
          @click="change('appearance', 'sidebar_position', p)"
        >
          {{ p === 'left' ? '左侧' : p === 'right' ? '右侧' : '顶部' }}
        </button>
      </div>
    </div>

    <!-- 悬停展开 -->
    <div class="card row-card">
      <div class="row-main">
        <div class="row-title">悬停展开侧边栏</div>
        <div class="row-desc">鼠标移到屏幕边缘时自动展开</div>
      </div>
      <Switch :model-value="settings.general.enable_mouse_hover" aria-label="悬停展开侧边栏" @update:model-value="change('general', 'enable_mouse_hover', $event)" />
    </div>

    <div class="card row-card">
      <div class="row-main">
        <div class="row-title">显示器</div>
        <div class="row-desc">选择侧边栏停靠屏幕；自动模式跟随鼠标所在屏幕</div>
      </div>
      <MonitorPicker
        :model-value="settings.appearance.sidebar_monitor_id ?? null"
        @update:model-value="(value) => change('appearance', 'sidebar_monitor_id', value)"
      />
    </div>

    <!-- 自动隐藏延迟 -->
    <div class="card row-card">
      <div class="row-main">
        <div class="row-title">自动隐藏延迟</div>
        <div class="row-desc">鼠标离开后 {{ settings.general.auto_hide_delay }} ms 收起</div>
      </div>
      <RangeSlider
        :model-value="settings.general.auto_hide_delay"
        :min="0"
        :max="10000"
        :step="100"
        :format="fmtMs"
        aria-label="自动隐藏延迟"
        @commit="(v) => change('general', 'auto_hide_delay', v)"
      />
    </div>

    <!-- 悬停预览延迟 -->
    <div class="card row-card">
      <div class="row-main">
        <div class="row-title">悬停预览延迟</div>
        <div class="row-desc">鼠标悬停插件图标 {{ settings.general.hover_preview_delay ?? 350 }} ms 后显示卡片</div>
      </div>
      <RangeSlider
        :model-value="settings.general.hover_preview_delay ?? 350"
        :min="50"
        :max="2000"
        :step="50"
        :format="fmtMs"
        aria-label="悬停预览延迟"
        @commit="(v) => change('general', 'hover_preview_delay', v)"
      />
    </div>
    <div class="card row-card">
      <div class="row-main">
        <div class="row-title">预览列表可见条数</div>
        <div class="row-desc">宿主表格预览默认显示 {{ settings.appearance.preview_visible_rows ?? 3 }} 条；自定义插件页由插件控制</div>
      </div>
      <RangeSlider
        :model-value="settings.appearance.preview_visible_rows ?? 3"
        :min="1"
        :max="8"
        :step="1"
        :format="fmtRows"
        aria-label="预览列表可见条数"
        @commit="(v) => change('appearance', 'preview_visible_rows', v)"
      />
    </div>
    <div class="card row-card">
      <div class="row-main">
        <div class="row-title">预览显示模式</div>
        <div class="row-desc">简洁模式隐藏插件标题区，保留状态和预览数据；自定义预览页可自行处理</div>
      </div>
      <div class="segmented">
        <button
          type="button"
          :class="{ on: (settings.appearance.preview_display_mode ?? 'default') === 'default' }"
          aria-label="默认预览显示模式"
          @click="change('appearance', 'preview_display_mode', 'default')"
        >默认</button>
        <button
          type="button"
          :class="{ on: settings.appearance.preview_display_mode === 'compact' }"
          aria-label="简洁预览显示模式"
          @click="change('appearance', 'preview_display_mode', 'compact')"
        >简洁</button>
      </div>
    </div>
    <div class="card row-card preview-width-card">
      <div class="row-main">
        <div class="row-title">预览卡片宽度</div>
        <div class="row-desc">
          {{ settings.appearance.preview_width == null ? '跟随插件清单默认宽度；自定义插件页由插件控制' : `${settings.appearance.preview_width} px（仅宿主表格预览）` }}
        </div>
      </div>
      <div class="preview-width-controls">
        <div class="segmented">
          <button
            type="button"
            :class="{ on: settings.appearance.preview_width == null }"
            aria-label="预览卡片宽度跟随插件默认值"
            @click="change('appearance', 'preview_width', null)"
          >跟随插件</button>
          <button
            type="button"
            :class="{ on: settings.appearance.preview_width != null }"
            aria-label="自定义预览卡片宽度"
            @click="change('appearance', 'preview_width', settings.appearance.preview_width ?? 280)"
          >自定义</button>
        </div>
        <RangeSlider
          v-if="settings.appearance.preview_width != null"
          :model-value="settings.appearance.preview_width"
          :min="240"
          :max="480"
          :step="20"
          :format="fmtPx"
          aria-label="预览卡片宽度"
          @commit="(v) => change('appearance', 'preview_width', v)"
        />
      </div>
    </div>
      </div>
    </section>

    <!-- 运行模式 -->
    <section class="settings-group">
      <div class="settings-group-heading"><div><h2>系统</h2><p>在响应速度、资源占用和后台启动之间选择。</p></div></div>
      <div class="settings-group-card">
    <div class="card row-card">
      <div class="row-main">
        <div class="row-title">运行模式</div>
        <div class="row-desc">{{ perfDesc }}</div>
      </div>
      <div class="segmented">
        <button
          v-for="p in ([
            { id: 'eco', label: '省资源' },
            { id: 'balanced', label: '平衡' },
            { id: 'performance', label: '极速' },
            { id: 'auto', label: '自动' }
          ] as const)"
          :key="p.id"
          :class="{ on: (settings.general.performance_profile ?? 'balanced') === p.id }"
          :aria-label="`运行模式：${p.label}`"
          @click="changePerformanceProfile(p.id)"
        >
          {{ p.label }}
        </button>
      </div>
    </div>

    <!-- 开机自启 -->
    <div class="card row-card">
      <div class="row-main">
        <div class="row-title">开机自启</div>
        <div class="row-desc">登录 Windows 后自动启动 SideWeave</div>
      </div>
      <Switch :model-value="autostartRef" aria-label="开机自启" @update:model-value="change('general', 'run_on_startup', $event)" />
    </div>
      </div>
    </section>

    <!-- 显示/隐藏侧边栏热键 -->
    <section class="settings-group">
      <div class="settings-group-heading"><div><h2>快捷键</h2><p>录制全局快捷键；留空表示停用。</p></div></div>
      <div class="settings-group-card">
    <div class="card row-card">
      <div class="row-main">
        <div class="row-title">显示 / 隐藏侧边栏热键</div>
        <div class="row-desc">全局热键，修改后立即生效</div>
      </div>
      <HotkeyRecorder
        :model-value="settings.shortcuts.toggle_sidebar"
        @commit="changeHotkey"
      />
    </div>

    <!-- 呼出控制中心热键 -->
    <div class="card row-card">
      <div class="row-main">
        <div class="row-title">呼出控制中心热键</div>
        <div class="row-desc">全局热键；留空 = 不启用</div>
      </div>
      <HotkeyRecorder
        :model-value="settings.shortcuts.control_center"
        @commit="changeControlCenterHotkey"
      />
    </div>
      </div>
    </section>

    <!-- 市场/云端：插件中心与皮肤中心入口（位于控制中心导航） -->
    <section class="settings-group">
      <div class="settings-group-heading"><div><h2>更多管理</h2><p>打开完整控制中心，管理插件、市场和系统诊断。</p></div></div>
      <div class="settings-group-card">
    <div class="card row-card more-card">
      <div class="row-main">
        <div class="row-title">市场 / 云端</div>
        <div class="row-desc">插件中心与皮肤中心在控制中心左侧导航</div>
      </div>
      <Button variant="default" size="sm" class="btn accent" type="button" @click="openControlCenter">
        <Icon name="blocks" :size="14" />
        打开市场
      </Button>
    </div>

    <!-- 更多设置：入口到完整控制中心 -->
    <div class="card row-card more-card">
      <div class="row-main">
        <div class="row-title">更多设置</div>
        <div class="row-desc">插件管理、系统信息与诊断在控制中心</div>
      </div>
      <Button variant="default" size="sm" class="btn accent" type="button" @click="openControlCenter">
        <Icon name="layout-dashboard" :size="14" />
        打开控制中心
      </Button>
    </div>
      </div>
    </section>

    <Toast :message="toast" :kind="toastKind" @done="toast = ''" />
  </div>
</template>

<style scoped>
.settings {
  max-width: 640px;
}

.card {
  background: color-mix(in srgb, var(--surface-subtle) calc(var(--component-opacity, 1) * 100%), transparent);
  border: 1px solid color-mix(in srgb, var(--border) 65%, transparent);
  border-radius: var(--radius, 12px);
  margin-bottom: 10px;
}

.row-card {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  padding: 12px 16px;
}

.preview-width-card {
  align-items: flex-start;
}

.preview-width-controls {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  flex-wrap: wrap;
  gap: 10px;
}

.row-main {
  min-width: 0;
}

.row-title {
  font-size: 13px;
  font-weight: 500;
}
.row-desc {
  margin-top: 2px;
  font-size: 11.5px;
  color: var(--text-muted);
  word-break: break-all;
}

/* "更多设置"入口卡片：强调色描边突出这是跳转行为 */
.more-card {
  border-color: color-mix(in srgb, var(--primary) 28%, var(--border));
}

.btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 0 12px;
  border-radius: var(--radius, 6px);
  border: 1px solid var(--border);
  background: var(--surface);
  color: var(--text);
  font-size: 13px;
  white-space: nowrap;
}
.btn:hover {
  border-color: var(--border-strong);
  color: var(--accent-foreground);
}
.btn.accent {
  border-color: transparent;
  background: var(--primary);
  color: var(--primary-foreground);
  font-weight: 600;
}
.btn.accent:hover:not(:disabled) {
  filter: brightness(1.08);
  color: var(--primary-foreground);
}

/* switch 样式（与控制中心一致） */
.switch {
  appearance: none;
  width: 40px;
  height: 22px;
  border-radius: 999px;
  background: var(--border);
  position: relative;
  cursor: pointer;
  transition: background 0.2s ease;
  flex: none;
}
.switch::after {
  content: '';
  position: absolute;
  top: 2px;
  left: 2px;
  width: 18px;
  height: 18px;
  border-radius: 50%;
  background: #fff;
  transition: transform 0.2s ease;
}
.switch:checked {
  background: var(--primary);
}
.switch:checked::after {
  transform: translateX(18px);
}

.segmented {
  display: flex;
  flex-wrap: wrap;
  justify-content: flex-end;
  gap: 1px;
  padding: 3px;
  border: 1px solid var(--border);
  border-radius: var(--radius-lg, 10px);
  background: var(--surface);
  flex: none;
}
.segmented button {
  padding: 5px 12px;
  border: none;
  border-radius: var(--radius, 6px);
  background: transparent;
  color: var(--text-muted);
  font-size: 12px;
}
.segmented button.on {
  background: var(--surface-subtle);
  color: var(--text);
  font-weight: 600;
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--border) 75%, transparent);
}

.swatches {
  display: flex;
  align-items: center;
  gap: 8px;
  flex: none;
}
.swatch {
  width: 24px;
  height: 24px;
  padding: 0;
  border-radius: 50%;
  border: 2px solid transparent;
  cursor: pointer;
}
.swatch.on {
  border-color: var(--text);
  box-shadow: 0 0 0 2px color-mix(in srgb, var(--text) 25%, transparent);
}
.swatch.custom {
  width: 28px;
  height: 28px;
  border: 1px dashed var(--border-strong);
  background: transparent;
  padding: 0;
}

.settings {
  display: flex;
  max-width: 760px;
  flex-direction: column;
  gap: 22px;
  margin: 0 auto;
  padding: 18px 22px 30px;
}
.settings-hero { display: flex; align-items: center; gap: 12px; padding: 2px 0 4px; }
.settings-hero-icon { display: grid; width: 36px; height: 36px; flex: none; place-items: center; border: 1px solid var(--border); border-radius: 8px; background: var(--muted); color: var(--foreground); }
.settings-hero-kicker { color: var(--muted-foreground); font-size: 9px; font-weight: 650; letter-spacing: .08em; }
.settings-hero h1 { margin: 2px 0 0; font-size: 18px; font-weight: 650; letter-spacing: -.03em; }
.settings-hero p,
.settings-group-heading p { margin: 4px 0 0; color: var(--muted-foreground); font-size: 10px; line-height: 1.45; }
.settings-group { display: flex; flex-direction: column; gap: 9px; }
.settings-group-heading h2 { margin: 0; font-size: 12px; font-weight: 600; }
.settings-group-card {
  overflow: hidden;
  border: 1px solid var(--border);
  border-radius: 8px;
  background: color-mix(in srgb, var(--card) calc(var(--component-opacity, 1) * 100%), transparent);
  box-shadow: 0 1px 2px rgb(0 0 0 / 3%);
}
.settings-group-card .row-card { min-height: 58px; margin: 0; padding: 12px 14px; border: 0; border-radius: 0; background: transparent; box-shadow: none; }
.settings-group-card .row-card + .row-card { border-top: 1px solid var(--border); }
.settings-group-card .row-title { font-size: 11px; font-weight: 550; }
.settings-group-card .row-desc { color: var(--muted-foreground); font-size: 10px; }
.settings-group-card .more-card { min-height: 64px; }
.settings-group-card .more-card:first-child { border-top: 0; }
.settings-group-card .btn { border-radius: 6px; }
.settings-group-card .switch { width: 34px; height: 19px; border: 1px solid var(--border); background: var(--input); }
.settings-group-card .switch::after { width: 15px; height: 15px; background: var(--background); box-shadow: 0 1px 2px rgb(0 0 0 / 15%); }
.settings-group-card .switch:checked::after { transform: translateX(15px); }
.settings-group-card .segmented { gap: 2px; padding: 3px; border-radius: 7px; background: var(--muted); }
.settings-group-card .segmented button { min-height: 27px; border-radius: 5px; background: transparent; font-size: 10px; }
.settings-group-card .segmented button.on { background: var(--background); box-shadow: 0 1px 2px rgb(0 0 0 / 8%); }
</style>
