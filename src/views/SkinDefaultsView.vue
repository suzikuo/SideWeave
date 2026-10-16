<script setup lang="ts">
import Button from '@/components/ui/button/Button.vue'
import { computed, ref, watch } from 'vue'
import { setSetting, setSkinAppearance } from '../lib/api'
import type { InstalledSkinAppearance, ResolvedSkinDefaults } from '../lib/skinAppearance'
import {
  applySkinWithDefaults,
  makeSkinDefaultsDraft,
  resolveSkinDefaults,
  skinPreferenceKey
} from '../lib/skinAppearance'
import { settingsRef } from '../lib/store'
import type { SidebarPosition, SkinAppearanceDefaults, ThemeMode } from '../lib/types'
import Icon from '../ui/Icon.vue'
import RangeSlider from '../ui/RangeSlider.vue'
import Toast from '../ui/Toast.vue'

interface SkinDefaultsForm extends ResolvedSkinDefaults {
  themeMode: ThemeMode
  accentColor: string
  sidebarPosition: SidebarPosition
  collapsedWidth: number
  iconSize: number
  peekWidth: number
}

const opacityControls = [
  { key: 'sidebarPanelOpacity', label: '侧边栏面板', description: '面板底色的不透明度' },
  { key: 'sidebarImageOpacity', label: '侧边栏背景图', description: '皮肤背景图片的可见度' },
  { key: 'detailPanelOpacity', label: '详情页面板', description: '详情页内容区域的面板底色' },
  { key: 'controlPanelOpacity', label: '控制中心面板', description: '控制中心独立于详情页调整' },
  { key: 'detailImageOpacity', label: '详情页背景图', description: '详情页与控制中心背景图片的可见度' },
  { key: 'componentOpacity', label: '卡片与组件', description: '卡片、按钮等组件表面的不透明度' }
] as const

const props = defineProps<{ skins: InstalledSkinAppearance[] }>()
const selectedSkinId = ref('default')
const selectionTouched = ref(false)
const draft = ref<SkinDefaultsForm | null>(null)
const busy = ref(false)
const toast = ref('')
const toastKind = ref<'info' | 'error' | 'success'>('info')

const skinOptions = computed(() => [
  { id: 'default', label: '默认皮肤' },
  ...props.skins.map((skin) => ({ id: `user:${skin.id}`, label: skin.name }))
])
const selectedName = computed(
  () => skinOptions.value.find((skin) => skin.id === selectedSkinId.value)?.label ?? selectedSkinId.value
)
const settings = computed(() => settingsRef.value)
const activeSkinId = computed(() => settings.value?.appearance.skin ?? 'default')
const isActiveSkin = computed(() => activeSkinId.value === selectedSkinId.value)
const hasCustomDefaults = computed(() => {
  const key = skinPreferenceKey(selectedSkinId.value)
  return Boolean(settings.value?.appearance.skin_defaults?.[key])
})

function showToast(message: string, kind: 'info' | 'error' | 'success' = 'info') {
  toastKind.value = kind
  toast.value = ''
  requestAnimationFrame(() => {
    toast.value = message
  })
}

function loadDraft() {
  const appearance = settings.value?.appearance
  if (!appearance) return
  const defaults = makeSkinDefaultsDraft(
    selectedSkinId.value,
    props.skins,
    appearance,
    appearance.skin_defaults ?? {}
  )
  draft.value = {
    ...defaults,
    themeMode: defaults.themeMode ?? appearance.theme_mode,
    accentColor: defaults.accentColor ?? appearance.accent_color,
    sidebarPosition: defaults.sidebarPosition ?? appearance.sidebar_position,
    collapsedWidth: defaults.collapsedWidth ?? appearance.collapsed_width,
    iconSize: defaults.iconSize ?? appearance.icon_size,
    peekWidth: defaults.peekWidth ?? appearance.peek_width
  }
}

watch(selectedSkinId, loadDraft)
watch(
  () => props.skins,
  () => {
    if (!selectionTouched.value || !skinOptions.value.some((skin) => skin.id === selectedSkinId.value)) {
      const active = activeSkinId.value
      selectedSkinId.value = skinOptions.value.some((skin) => skin.id === active) ? active : 'default'
    }
    loadDraft()
  },
  { deep: true, immediate: true }
)
watch(
  () => settings.value?.appearance.skin_defaults,
  loadDraft,
  { deep: true }
)

function updateDraft<K extends keyof SkinDefaultsForm>(key: K, value: SkinDefaultsForm[K]) {
  if (draft.value) draft.value = { ...draft.value, [key]: value }
}

function defaultsForSave(): SkinAppearanceDefaults {
  if (!draft.value) throw new Error('皮肤参数尚未加载')
  const {
    sidebarPanelOpacity,
    detailPanelOpacity,
    controlPanelOpacity,
    sidebarImageOpacity,
    detailImageOpacity,
    componentOpacity,
    themeMode,
    accentColor,
    sidebarPosition,
    collapsedWidth,
    iconSize,
    peekWidth
  } = draft.value
  return {
    sidebarPanelOpacity,
    detailPanelOpacity,
    controlPanelOpacity,
    sidebarImageOpacity,
    detailImageOpacity,
    componentOpacity,
    themeMode,
    accentColor,
    sidebarPosition,
    collapsedWidth,
    iconSize,
    peekWidth
  }
}

async function saveDefaults() {
  if (!draft.value || busy.value || !settingsRef.value) return
  busy.value = true
  try {
    const key = skinPreferenceKey(selectedSkinId.value)
    const nextDefaults = {
      ...settingsRef.value.appearance.skin_defaults,
      [key]: defaultsForSave()
    }
    settingsRef.value = await setSetting('appearance', 'skin_defaults', nextDefaults)
    if (isActiveSkin.value) {
      settingsRef.value = await setSkinAppearance(selectedSkinId.value, defaultsForSave())
    }
    showToast(isActiveSkin.value ? '默认参数已保存并应用' : '皮肤默认参数已保存', 'success')
  } catch (error) {
    showToast(String(error), 'error')
  } finally {
    busy.value = false
  }
}

async function applyDefaults() {
  if (busy.value || !settingsRef.value) return
  busy.value = true
  try {
    settingsRef.value = await applySkinWithDefaults(
      selectedSkinId.value,
      props.skins,
      settingsRef.value.appearance.skin_defaults ?? {}
    )
    showToast(`已应用“${selectedName.value}”的默认参数`, 'success')
  } catch (error) {
    showToast(String(error), 'error')
  } finally {
    busy.value = false
  }
}

async function restoreRecommended() {
  if (busy.value || !settingsRef.value) return
  busy.value = true
  try {
    const key = skinPreferenceKey(selectedSkinId.value)
    const nextDefaults = { ...settingsRef.value.appearance.skin_defaults }
    delete nextDefaults[key]
    settingsRef.value = await setSetting('appearance', 'skin_defaults', nextDefaults)
    draft.value = null
    loadDraft()
    if (isActiveSkin.value) {
      const recommended = resolveSkinDefaults(selectedSkinId.value, props.skins)
      settingsRef.value = await setSkinAppearance(selectedSkinId.value, recommended)
    }
    showToast('已恢复皮肤推荐参数', 'success')
  } catch (error) {
    showToast(String(error), 'error')
  } finally {
    busy.value = false
  }
}

const fmtPercent = (value: number) => `${Math.round(value * 100)}%`
const fmtPx = (value: number) => `${value} px`
</script>

<template>
  <div class="skin-defaults">
    <div class="defaults-intro">
      <div>
        <h3>每款皮肤的默认参数</h3>
        <p>保存后，切换到该皮肤或点击“应用默认参数”时会使用这里的设置。透明度推荐值来自皮肤包。</p>
      </div>
      <label class="skin-picker">
        <span>选择皮肤</span>
        <select v-model="selectedSkinId" @change="selectionTouched = true">
          <option v-for="skin in skinOptions" :key="skin.id" :value="skin.id">{{ skin.label }}</option>
        </select>
      </label>
    </div>

    <template v-if="draft">
      <div class="defaults-group">
        <h4>透明度</h4>
        <div v-for="control in opacityControls" :key="control.key" class="card row-card">
          <div class="row-main">
            <div class="row-title">{{ control.label }}</div>
            <div class="row-desc">{{ control.description }}</div>
          </div>
          <RangeSlider
            :model-value="draft[control.key]"
            :min="0"
            :max="1"
            :step="0.05"
            :format="fmtPercent"
            :aria-label="`${selectedName} ${control.label}`"
            @commit="(value) => updateDraft(control.key, value)"
          />
        </div>
      </div>

      <div class="defaults-group">
        <h4>常用外观</h4>
        <div class="card row-card">
          <div class="row-main">
            <div class="row-title">主题模式</div>
            <div class="row-desc">应用这款皮肤时采用的浅色、深色或系统模式</div>
          </div>
          <div class="segmented">
            <button v-for="mode in (['light', 'dark', 'system'] as const)" :key="mode" :class="{ on: draft.themeMode === mode }" @click="updateDraft('themeMode', mode)">
              {{ mode === 'light' ? '浅色' : mode === 'dark' ? '深色' : '跟随系统' }}
            </button>
          </div>
        </div>
        <div class="card row-card">
          <div class="row-main">
            <div class="row-title">强调色</div>
            <div class="row-desc">皮肤应用时采用的主强调颜色</div>
          </div>
          <input class="swatch custom" type="color" :value="draft.accentColor" :aria-label="`${selectedName}强调色`" @change="updateDraft('accentColor', ($event.target as HTMLInputElement).value)" />
        </div>
        <div class="card row-card">
          <div class="row-main">
            <div class="row-title">侧边栏位置</div>
            <div class="row-desc">选择皮肤默认停靠在屏幕的哪一侧</div>
          </div>
          <div class="segmented">
            <button v-for="position in (['left', 'right', 'top'] as const)" :key="position" :class="{ on: draft.sidebarPosition === position }" @click="updateDraft('sidebarPosition', position)">
              {{ position === 'left' ? '左侧' : position === 'right' ? '右侧' : '顶部' }}
            </button>
          </div>
        </div>
        <div class="card row-card">
          <div class="row-main">
            <div class="row-title">收起宽度</div>
            <div class="row-desc">{{ draft.collapsedWidth }} px</div>
          </div>
          <RangeSlider :model-value="draft.collapsedWidth" :min="56" :max="96" :step="2" :format="fmtPx" aria-label="皮肤默认收起宽度" @commit="(value) => updateDraft('collapsedWidth', value)" />
        </div>
        <div class="card row-card">
          <div class="row-main">
            <div class="row-title">图标大小</div>
            <div class="row-desc">{{ draft.iconSize }} px</div>
          </div>
          <RangeSlider :model-value="draft.iconSize" :min="20" :max="72" :step="2" :format="fmtPx" aria-label="皮肤默认图标大小" @commit="(value) => updateDraft('iconSize', value)" />
        </div>
        <div class="card row-card">
          <div class="row-main">
            <div class="row-title">边缘展开宽度</div>
            <div class="row-desc">{{ draft.peekWidth }} px</div>
          </div>
          <RangeSlider :model-value="draft.peekWidth" :min="0" :max="10" :step="1" :format="fmtPx" aria-label="皮肤默认边缘展开宽度" @commit="(value) => updateDraft('peekWidth', value)" />
        </div>
      </div>

      <div class="defaults-actions">
        <Button variant="outline" size="sm" class="btn" :disabled="busy || !hasCustomDefaults" @click="restoreRecommended">
          <Icon name="refresh" :size="14" /> 恢复推荐值
        </Button>
        <Button variant="outline" size="sm" class="btn" :disabled="busy || isActiveSkin" @click="applyDefaults">
          <Icon name="check" :size="14" /> {{ isActiveSkin ? '当前正在使用' : '应用默认参数' }}
        </Button>
        <Button variant="default" size="sm" class="btn accent" :disabled="busy" @click="saveDefaults">
          <Icon name="check" :size="14" /> {{ busy ? '保存中…' : isActiveSkin ? '保存并应用' : '保存默认值' }}
        </Button>
      </div>
    </template>

    <Toast :message="toast" :kind="toastKind" @done="toast = ''" />
  </div>
</template>

<style scoped>
.skin-defaults {
  display: flex;
  flex-direction: column;
  gap: 14px;
  min-height: 0;
}

.defaults-intro {
  display: flex;
  align-items: flex-end;
  justify-content: space-between;
  gap: 16px;
  padding: 3px 2px 7px;
}

.defaults-intro h3,
.defaults-group h4 {
  margin: 0;
  color: var(--text);
  font-size: 14px;
  font-weight: 650;
}

.defaults-intro p {
  max-width: 560px;
  margin: 6px 0 0;
  color: var(--text-muted);
  font-size: 11.5px;
  line-height: 1.6;
}

.skin-picker {
  display: grid;
  flex: none;
  gap: 5px;
  min-width: 180px;
  color: var(--text-muted);
  font-size: 11px;
}

.skin-picker select {
  min-height: 34px;
  padding: 0 10px;
  border: 1px solid var(--border);
  border-radius: 9px;
  outline: none;
  background: var(--surface);
  color: var(--text);
}

.skin-picker select:focus-visible {
  border-color: var(--accent);
}

.defaults-group {
  display: grid;
  gap: 8px;
}

.defaults-group h4 {
  padding: 5px 2px 1px;
  color: var(--text-muted);
  font-size: 11px;
  letter-spacing: 0.04em;
  text-transform: uppercase;
}

.defaults-actions {
  display: flex;
  flex-wrap: wrap;
  justify-content: flex-end;
  gap: 8px;
  padding: 8px 0 4px;
}

@media (max-width: 680px) {
  .defaults-intro {
    align-items: stretch;
    flex-direction: column;
  }

  .skin-picker {
    width: 100%;
  }
}
</style>
