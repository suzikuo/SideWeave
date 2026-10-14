<script setup lang="ts">
/** 插件中心：浏览 SideWeave 市场，安装 / 更新 / 打开 / 移除独立插件。 */
import { computed, onMounted, onUnmounted, ref } from 'vue'
import Icon from '../ui/Icon.vue'
import Toast from '../ui/Toast.vue'
import {
  marketInstall,
  marketInstallInfo,
  marketInstalled,
  pluginImportLocal,
  marketRefresh,
  marketSnapshot,
  marketUninstall,
  marketUpdatedAt
} from '../lib/marketApi'
import type { InstalledItem, MarketIndex, MarketItem } from '../lib/marketApi'
import { openPluginDetail } from '../lib/api'

const KIND = 'plugin' as const

const index = ref<MarketIndex | null>(null)
const installed = ref<InstalledItem[]>([])
const loading = ref(false)
const error = ref('')
/** 安装/更新在途标记（id → true） */
const installing = ref<Record<string, boolean>>({})
/** 两次点击确认移除（id → true） */
const confirmRemove = ref<Record<string, boolean>>({})

const toast = ref('')
const toastKind = ref<'info' | 'error' | 'success'>('info')
function showToast(message: string, kind: 'info' | 'error' | 'success' = 'info') {
  toastKind.value = kind
  toast.value = ''
  requestAnimationFrame(() => {
    toast.value = message
  })
}

const updatedAtText = computed(() => marketUpdatedAt(index.value))

const installedMap = computed(() => {
  const map = new Map<string, InstalledItem>()
  for (const it of installed.value) map.set(it.id, it)
  return map
})

function installedOf(id: string): InstalledItem | undefined {
  return installedMap.value.get(id)
}

function isInstalling(id: string): boolean {
  return Boolean(installing.value[id])
}

/** 可用插件：同一 ID 只在一个列表显示；已安装插件的更新入口统一放在下方 */
const available = computed<MarketItem[]>(() => {
  const items = index.value?.plugins ?? []
  return items.filter((m) => {
    const inst = installedOf(m.id)
    return !inst
  })
})

const importingLocal = ref(false)

/** 本地离线安装：选择 zip 包，不经过市场 */
async function importLocal() {
  if (importingLocal.value) return
  importingLocal.value = true
  try {
    await pluginImportLocal(KIND)
    showToast('本地插件包已安装并加载完成', 'success')
    await reloadInstalled()
  } catch (e) {
    showToast(String(e), 'error')
  } finally {
    importingLocal.value = false
  }
}

async function reloadInstalled() {
  installed.value = (await marketInstalled()).plugins
}

async function loadCached() {
  try {
    const [idx, inst] = await Promise.all([marketSnapshot(), marketInstalled()])
    index.value = idx
    installed.value = inst.plugins
    error.value = ''
  } catch {
    // First launch may not have a cache yet; the background refresh below fills it.
  }
}

async function refreshMarket(showSpinner: boolean) {
  if (showSpinner) loading.value = true
  try {
    const [idx, inst] = await Promise.all([marketRefresh(), marketInstalled()])
    index.value = idx
    installed.value = inst.plugins
  } catch (e) {
    if (!index.value) error.value = String(e)
  } finally {
    loading.value = false
  }
}

async function reload() {
  await loadCached()
  await refreshMarket(true)
}

onMounted(async () => {
  window.addEventListener('sideweave:market-install', onDeepLinkInstall)
  await loadCached()
  void refreshMarket(false)
})
onUnmounted(() => window.removeEventListener('sideweave:market-install', onDeepLinkInstall))

function onDeepLinkInstall(event: Event) {
  const detail = (event as CustomEvent<{ kind: string; id: string }>).detail
  if (detail?.kind === KIND && /^[a-z0-9_-]{1,64}$/.test(detail.id)) {
    void installById(detail.id, detail.id)
  }
}

/** 安装 / 更新（同一命令，下载耗时） */
async function installById(id: string, name: string) {
  if (isInstalling(id)) return
  installing.value = { ...installing.value, [id]: true }
  try {
    const info = await marketInstallInfo(KIND, id)
    let trustApproval: { publisher: string; keyId: string; keyFingerprint: string } | undefined
    if (info.native && (info.trustRequired || !info.verified)) {
      const capabilities = info.capabilities.length ? info.capabilities.join(', ') : '未声明'
      const review = info.verified ? '' : '\n\n此条目未经市场维护者审核。'
      const trust = info.trustRequired
        ? `\n\n首次信任此发布者/密钥：${info.publisherName} (${info.publisher})\nMinisign 指纹：${info.keyFingerprint}`
        : ''
      const accepted = window.confirm(
        `「${name}」是 native 插件，将以本机权限运行。\n发布者：${info.publisherName} (${info.publisher})\n能力声明：${capabilities}${trust}${review}\n\nNative 代码可以绕过宿主 API 直接访问系统；能力声明用于展示和审查，不构成系统级沙箱。你信任这个发布者吗？`
      )
      if (!accepted) return
      trustApproval = {
        publisher: info.publisher,
        keyId: info.keyId,
        keyFingerprint: info.keyFingerprint
      }
    }
    await marketInstall(KIND, id, trustApproval)
    showToast(`「${name}」安装 / 更新完成`, 'success')
    await reloadInstalled()
  } catch (e) {
    showToast(String(e), 'error')
  } finally {
    installing.value = { ...installing.value, [id]: false }
  }
}

function install(item: MarketItem) {
  void installById(item.id, item.name)
}

/** 已安装列表的更新入口 */
function updateInstalled(item: InstalledItem) {
  void installById(item.id, item.name)
}

/** 打开已安装插件：所有运行时统一交给详情宿主解析。 */
async function openInstalled(it: InstalledItem) {
  try {
    await openPluginDetail(it.id)
  } catch (e) {
    showToast(String(e), 'error')
  }
}

/** 移除（两次点击确认） */
async function removeInstalled(item: InstalledItem) {
  if (!confirmRemove.value[item.id]) {
    confirmRemove.value = { ...confirmRemove.value, [item.id]: true }
    window.setTimeout(() => {
      confirmRemove.value = { ...confirmRemove.value, [item.id]: false }
    }, 3000)
    return
  }
  confirmRemove.value = { ...confirmRemove.value, [item.id]: false }
  try {
    await marketUninstall(KIND, item.id)
    showToast(`「${item.name}」已移除`, 'success')
    await reloadInstalled()
  } catch (e) {
    showToast(String(e), 'error')
  }
}
</script>

<template>
  <div class="market">
    <!-- 顶部：统计 chip + 刷新 -->
    <div class="toolbar">
      <div class="chips">
        <span class="chip accent">已安装 {{ installed.length }}</span>
        <span v-if="updatedAtText" class="chip" title="市场目录更新时间">目录：{{ updatedAtText }}</span>
      </div>
      <div class="btn-group">
        <button class="btn" :disabled="loading || importingLocal" @click="importLocal">
          <Icon name="plus" :size="14" /> 本地导入
        </button>
        <button class="btn" :disabled="loading" @click="reload">
          <Icon name="refresh" :size="14" :class="{ spin: loading }" />
          刷新
        </button>
      </div>
    </div>

    <!-- 加载中 -->
    <div v-if="loading" class="state">
      <Icon name="refresh" :size="22" class="spin" />
      <p class="state-title">正在加载市场…</p>
    </div>

    <!-- 加载失败 -->
    <div v-else-if="error" class="state">
      <Icon name="info" :size="22" />
      <p class="state-title">市场加载失败</p>
      <p class="state-desc">{{ error }}</p>
      <button class="btn" @click="reload"><Icon name="refresh" :size="14" /> 重试</button>
    </div>

    <!-- 列表区 -->
    <div v-else class="lists">
      <h3 class="group-label">可用插件（{{ available.length }}）</h3>
      <p v-if="available.length === 0" class="empty-row">云端没有可安装的插件</p>
      <div v-for="m in available" :key="m.id" class="item">
        <div class="item-main">
          <div class="item-title">
            <strong>{{ m.name }}</strong>
            <span class="ver-badge">v{{ m.latest }}</span>
            <span v-if="m.type === 'native'" class="chip warn" title="Native 插件拥有本机代码执行能力">Native Beta</span>
            <span v-if="m.verified" class="chip">已审核</span>
            <span v-else class="chip warn">未审核</span>
            <span v-if="m.publisher" class="item-author">{{ m.publisher }}</span>
          </div>
          <p class="item-desc">{{ m.description || '暂无描述' }}</p>
          <p v-if="m.repository" class="item-source">来源：{{ m.repository }}</p>
        </div>
        <div class="item-actions">
          <button
            v-if="!installedOf(m.id)"
            class="btn accent"
            :disabled="isInstalling(m.id)"
            @click="install(m)"
          >
            <Icon v-if="isInstalling(m.id)" name="refresh" :size="13" class="spin" />
            <Icon v-else name="plus" :size="13" />
            {{ isInstalling(m.id) ? '安装中…' : '安装' }}
          </button>
          <span class="chip">可安装</span>
        </div>
      </div>

      <h3 class="group-label">已安装（{{ installed.length }}）</h3>
      <p v-if="installed.length === 0" class="empty-row">还没有安装任何插件</p>
      <div v-for="it in installed" :key="it.id" class="item">
        <div class="item-main">
          <div class="item-title">
            <strong>{{ it.name }}</strong>
            <span class="ver-badge">v{{ it.version }}</span>
            <span v-if="it.runtime === 'dynamic_library'" class="chip" title="可信本地 DLL 插件">本地 DLL</span>
            <span v-else-if="it.runtime === 'external_process'" class="chip" title="独立进程插件">独立进程</span>
            <span v-if="it.updateVersion" class="chip warn">有更新</span>
          </div>
          <p class="item-desc">{{ it.description || '暂无描述' }}</p>
        </div>
        <div class="item-actions">
          <button class="btn" @click="openInstalled(it)">
            <Icon name="play" :size="13" /> 打开
          </button>
          <button
            v-if="it.updateVersion"
            class="btn accent"
            :disabled="isInstalling(it.id)"
            :title="`更新到 ${it.updateVersion}`"
            @click="updateInstalled(it)"
          >
            <Icon v-if="isInstalling(it.id)" name="refresh" :size="13" class="spin" />
            {{ isInstalling(it.id) ? '更新中…' : '更新' }}
          </button>
          <button
            class="btn danger"
            :class="{ confirming: confirmRemove[it.id] }"
            @click="removeInstalled(it)"
          >
            <Icon name="trash" :size="13" />
            {{ confirmRemove[it.id] ? '确认移除' : '移除' }}
          </button>
        </div>
      </div>
    </div>

    <Toast :message="toast" :kind="toastKind" @done="toast = ''" />
  </div>
</template>

<style scoped>
.market {
  display: flex;
  flex-direction: column;
  height: 100%;
  gap: 10px;
}

/* ---------- 顶部工具条 ---------- */

.toolbar {
  flex: none;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.chips {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
  min-width: 0;
}

.chip {
  padding: 3px 10px;
  border: 1px solid color-mix(in srgb, var(--border) 70%, transparent);
  border-radius: 999px;
  background: var(--surface-subtle);
  color: var(--text-muted);
  font-size: 11px;
  font-weight: 600;
  white-space: nowrap;
}
.chip.accent {
  border-color: transparent;
  background: color-mix(in srgb, var(--accent) 14%, transparent);
  color: var(--accent);
}
.chip.warn {
  border-color: transparent;
  background: color-mix(in srgb, var(--warning) 16%, transparent);
  color: var(--warning);
}

/* ---------- 按钮 ---------- */

.btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 7px 14px;
  border-radius: 10px;
  border: 1px solid var(--border);
  background: var(--surface);
  color: var(--text);
  font-size: 12.5px;
  white-space: nowrap;
}
.btn:hover:not(:disabled) {
  border-color: var(--accent);
  color: var(--accent);
}
.btn:disabled {
  opacity: 0.55;
  cursor: default;
}
.btn.accent {
  border-color: transparent;
  background: var(--accent);
  color: #fff;
  font-weight: 600;
}
.btn.accent:hover:not(:disabled) {
  filter: brightness(1.1);
  color: #fff;
}
.btn.danger {
  color: var(--danger);
  border-color: color-mix(in srgb, var(--danger) 45%, transparent);
}
.btn.danger:hover:not(:disabled) {
  border-color: var(--danger);
  color: var(--danger);
}
.btn.danger.confirming {
  background: var(--danger);
  border-color: var(--danger);
  color: #fff;
  font-weight: 600;
}

.btn-group {
  display: inline-flex;
  gap: 8px;
}

/* ---------- 列表区 ---------- */

.lists {
  flex: 1;
  min-height: 0;
  overflow: auto;
  overscroll-behavior: contain;
  padding-right: 2px;
}

.group-label {
  margin: 4px 2px 8px;
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.04em;
  color: var(--text-muted);
}

.item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 14px;
  padding: 12px 14px;
  margin-bottom: 8px;
  border: 1px solid color-mix(in srgb, var(--border) 65%, transparent);
  border-radius: var(--radius, 12px);
  background: var(--surface-subtle);
}

.item-main {
  min-width: 0;
}

.item-title {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
  font-size: 13px;
}

.item-author {
  font-size: 11px;
  color: var(--text-muted);
}

.item-desc {
  margin: 3px 0 0;
  font-size: 11.5px;
  color: var(--text-muted);
  word-break: break-all;
}

.ver-badge {
  padding: 1.5px 8px;
  border-radius: 999px;
  background: color-mix(in srgb, var(--accent) 13%, transparent);
  color: var(--accent);
  font-size: 10px;
  font-weight: 600;
  white-space: nowrap;
}

.item-actions {
  flex: none;
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 6px;
  flex-wrap: wrap;
}

.empty-row {
  padding: 16px 0;
  margin-bottom: 8px;
  border: 1px dashed color-mix(in srgb, var(--border) 60%, transparent);
  border-radius: var(--radius, 12px);
  text-align: center;
  color: var(--text-muted);
  font-size: 12px;
}

/* ---------- 全宽状态（加载 / 失败 / 引导） ---------- */

.state {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 6px;
  text-align: center;
  color: var(--text-muted);
}
.state-title {
  margin: 4px 0 0;
  font-size: 13.5px;
  font-weight: 600;
  color: var(--text);
}
.state-desc {
  margin: 0 0 8px;
  max-width: 440px;
  font-size: 12px;
  line-height: 1.6;
}

.spin {
  animation: spin 0.9s linear infinite;
}
@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}
</style>
