<script setup lang="ts">
/** 皮肤中心：浏览 SideWeave 市场，安装 / 应用 / 更新 / 移除用户皮肤 */
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import Icon from '../ui/Icon.vue'
import Toast from '../ui/Toast.vue'
import MarketItemDetail from './MarketItemDetail.vue'
import { applySkinWithRecommendedOpacity } from '../lib/skinAppearance'
import { settingsRef } from '../lib/store'
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

const KIND = 'skin' as const

const index = ref<MarketIndex | null>(null)
const installed = ref<InstalledItem[]>([])
const loading = ref(false)
const error = ref('')
/** 安装/更新在途标记（id → true） */
const installing = ref<Record<string, boolean>>({})
/** 两次点击确认移除（id → true） */
const confirmRemove = ref<Record<string, boolean>>({})
const selectedItem = ref<MarketItem | null>(null)

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

/** 当前应用中的皮肤（appearance.skin，用户皮肤形如 "user:<id>"） */
const currentSkin = computed(() => settingsRef.value?.appearance.skin ?? 'default')

function isCurrent(id: string): boolean {
  return currentSkin.value === `user:${id}`
}

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

async function importLocal() {
  if (importingLocal.value) return
  importingLocal.value = true
  try {
    await pluginImportLocal(KIND)
    showToast('本地皮肤包已导入', 'success')
    await reloadInstalled()
  } catch (e) {
    showToast(String(e), 'error')
  } finally {
    importingLocal.value = false
  }
}

type MarketTab = 'discover' | 'installed' | 'updates'
type MarketView = 'grid' | 'list'
type SortMode = 'directory' | 'name-asc' | 'name-desc'

const activeTab = ref<MarketTab>('discover')
const searchQuery = ref('')
const sortMode = ref<SortMode>('directory')
const importingLocal = ref(false)

function readSavedView(): MarketView {
  try {
    return window.localStorage.getItem('sideweave.market.skin.view') === 'list' ? 'list' : 'grid'
  } catch {
    return 'grid'
  }
}

const viewMode = ref<MarketView>(readSavedView())
watch(viewMode, (value) => {
  try {
    window.localStorage.setItem('sideweave.market.skin.view', value)
  } catch {
    // The view switch remains usable when browser storage is unavailable.
  }
})

const catalogItems = computed<MarketItem[]>(() => index.value?.skins ?? [])
const updateCount = computed(() => installed.value.filter((item) => Boolean(item.updateVersion)).length)

function matchesSearch(query: string, fields: Array<string | undefined>): boolean {
  if (!query) return true
  const normalized = query.trim().toLocaleLowerCase()
  return fields.some((field) => field?.toLocaleLowerCase().includes(normalized))
}

function sortByName<T extends { name: string }>(items: T[]): T[] {
  if (sortMode.value === 'directory') return items
  const direction = sortMode.value === 'name-asc' ? 1 : -1
  return [...items].sort((a, b) => direction * a.name.localeCompare(b.name, 'zh-CN'))
}

const visibleCatalog = computed(() => sortByName(catalogItems.value.filter((item) => matchesSearch(searchQuery.value, [
  item.name, item.id, item.description, item.author, item.publisher, item.repository
]))))

const visibleInstalled = computed(() => sortByName(installed.value.filter((item) => {
  if (activeTab.value === 'updates' && !item.updateVersion) return false
  return matchesSearch(searchQuery.value, [
    item.name, item.id, item.description, item.author, item.version, item.updateVersion ?? ''
  ])
})))

const visibleCount = computed(() => activeTab.value === 'discover' ? visibleCatalog.value.length : visibleInstalled.value.length)
const tabTitle = computed(() => activeTab.value === 'discover' ? '发现皮肤' : activeTab.value === 'installed' ? '已安装皮肤' : '可更新皮肤')

async function reloadInstalled() {
  installed.value = (await marketInstalled()).skins
}

async function loadCached() {
  try {
    const [idx, inst] = await Promise.all([marketSnapshot(), marketInstalled()])
    index.value = idx
    installed.value = inst.skins
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
    installed.value = inst.skins
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
    void openDeepLinkInstall(detail.id)
  }
}

function itemFromInstalled(item: InstalledItem): MarketItem {
  return {
    id: item.id,
    name: item.name,
    description: item.description,
    author: item.author,
    publisher: '',
    type: 'skin',
    repository: '',
    verified: false,
    latest: item.updateVersion || item.version,
    capabilities: [],
    versions: []
  }
}

function openDetails(item: MarketItem, confirmInstall = false) {
  selectedItem.value = item
  if (confirmInstall) {
    void nextTick(() => installById(item.id, item.name))
  }
}

function openInstalledDetails(item: InstalledItem) {
  openDetails(catalogItems.value.find((entry) => entry.id === item.id) ?? itemFromInstalled(item))
}

async function openDeepLinkInstall(id: string) {
  let item = catalogItems.value.find((entry) => entry.id === id)
  if (!item) {
    await refreshMarket(false)
    item = catalogItems.value.find((entry) => entry.id === id)
  }
  if (!item) {
    showToast(`市场中没有找到皮肤「${id}」`, 'error')
    return
  }
  openDetails(item, true)
}

/** 安装 / 更新（同一命令，下载耗时） */
async function installById(id: string, name: string) {
  if (isInstalling(id)) return
  installing.value = { ...installing.value, [id]: true }
  try {
    const info = await marketInstallInfo(KIND, id)
    const review = info.verified ? '' : '\n市场维护者尚未审核此条目。'
    const accepted = window.confirm(
      `即将安装皮肤「${info.name}」v${info.version}\n发布者：${info.publisherName} (${info.publisher})${review}\n\n继续安装？`
    )
    if (!accepted) return
    await marketInstall(KIND, id)
    showToast(`「${name}」安装完成，可点击「应用」启用`, 'success')
    await reloadInstalled()
  } catch (e) {
    showToast(String(e), 'error')
  } finally {
    installing.value = { ...installing.value, [id]: false }
  }
}

function install(item: MarketItem) {
  openDetails(item, true)
}

/** 已安装列表的更新入口 */
function updateInstalled(item: InstalledItem) {
  void installById(item.id, item.name)
}

function updateCatalogItem(item: MarketItem) {
  const current = installedOf(item.id)
  if (current?.updateVersion) openDetails(item, true)
}

function applyCatalogItem(item: MarketItem) {
  openDetails(item)
}

/** 应用皮肤：appearance.skin = "user:<id>" */
async function applySkin(item: InstalledItem) {
  if (isCurrent(item.id)) return
  try {
    settingsRef.value = await applySkinWithRecommendedOpacity(`user:${item.id}`, installed.value)
    showToast('皮肤已应用', 'success')
  } catch (e) {
    showToast(String(e), 'error')
  }
}

function applySelectedSkin() {
  const item = selectedItem.value && installedOf(selectedItem.value.id)
  if (item) void applySkin(item)
}

function removeSelectedSkin() {
  const item = selectedItem.value && installedOf(selectedItem.value.id)
  if (item) void removeInstalled(item)
}

/** 移除（两次点击确认）；移除当前应用中的皮肤先回退默认皮肤 */
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
    if (isCurrent(item.id)) {
      settingsRef.value = await applySkinWithRecommendedOpacity('default', [])
    }
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
    <MarketItemDetail
      v-if="selectedItem"
      :item="selectedItem"
      :kind="KIND"
      :installed="installedOf(selectedItem.id)"
      :busy="isInstalling(selectedItem.id)"
      :remove-confirm="confirmRemove[selectedItem.id]"
      :current="isCurrent(selectedItem.id)"
      @back="selectedItem = null"
      @install="install(selectedItem)"
      @update="updateCatalogItem(selectedItem)"
      @apply="applySelectedSkin"
      @remove="removeSelectedSkin"
      @open-repository-error="showToast($event, 'error')"
    />
    <template v-else>
    <header class="market-heading">
      <div class="market-summary">
        <span class="chip accent">目录 {{ catalogItems.length }}</span>
        <span class="chip">已安装 {{ installed.length }}</span>
        <span v-if="updateCount" class="chip warn">待更新 {{ updateCount }}</span>
        <span v-if="updatedAtText" class="chip updated-time" title="市场目录更新时间">目录更新：{{ updatedAtText }}</span>
      </div>
      <div class="heading-actions">
        <button class="btn" :disabled="loading || importingLocal" @click="importLocal"><Icon name="plus" :size="14" />本地导入</button>
        <button class="btn" :disabled="loading" @click="reload"><Icon name="refresh" :size="14" :class="{ spin: loading }" />刷新目录</button>
      </div>
    </header>

    <section class="market-controls" aria-label="皮肤市场工具">
      <label class="search-field">
        <Icon name="search" :size="16" />
        <input v-model="searchQuery" type="search" placeholder="搜索皮肤名称、描述或发布者…" aria-label="搜索皮肤" />
        <button v-if="searchQuery" class="clear-search" type="button" aria-label="清除搜索" @click="searchQuery = ''"><Icon name="x" :size="14" /></button>
      </label>
      <div class="control-actions">
        <select v-model="sortMode" class="select-control sort-control" aria-label="皮肤排序">
          <option value="directory">目录顺序</option><option value="name-asc">名称 A–Z</option><option value="name-desc">名称 Z–A</option>
        </select>
        <div class="view-toggle" role="group" aria-label="展示方式">
          <button class="view-button" :class="{ active: viewMode === 'grid' }" type="button" aria-label="卡片视图" title="卡片视图" :aria-pressed="viewMode === 'grid'" @click="viewMode = 'grid'"><Icon name="grid" :size="15" /></button>
          <button class="view-button" :class="{ active: viewMode === 'list' }" type="button" aria-label="列表视图" title="列表视图" :aria-pressed="viewMode === 'list'" @click="viewMode = 'list'"><Icon name="list" :size="16" /></button>
        </div>
      </div>
    </section>

    <nav class="market-tabs" aria-label="皮肤分类">
      <button class="market-tab" :class="{ active: activeTab === 'discover' }" @click="activeTab = 'discover'">发现 <span>{{ catalogItems.length }}</span></button>
      <button class="market-tab" :class="{ active: activeTab === 'installed' }" @click="activeTab = 'installed'">已安装 <span>{{ installed.length }}</span></button>
      <button class="market-tab" :class="{ active: activeTab === 'updates' }" @click="activeTab = 'updates'">有更新 <span :class="{ 'count-warn': updateCount }">{{ updateCount }}</span></button>
    </nav>

    <div v-if="loading" class="state"><Icon name="refresh" :size="22" class="spin" /><p class="state-title">正在加载市场…</p></div>
    <div v-else-if="error" class="state">
      <Icon name="info" :size="22" /><p class="state-title">市场加载失败</p><p class="state-desc">{{ error }}</p>
      <button class="btn" @click="reload"><Icon name="refresh" :size="14" />重试</button>
    </div>

    <section v-else class="market-content" aria-live="polite">
      <div class="results-heading"><div><h3>{{ tabTitle }}</h3><span>显示 {{ visibleCount }} 项</span></div><span v-if="searchQuery" class="filter-summary">搜索结果</span></div>
      <div v-if="activeTab === 'discover'" class="results" :class="[viewMode, activeTab]">
        <article v-for="item in visibleCatalog" :key="item.id" class="market-item market-card" tabindex="0" @click="openDetails(item)" @keydown.enter="openDetails(item)">
          <div class="item-head">
            <div class="product-icon"><Icon name="palette" :size="21" /></div>
            <div class="item-main">
              <div class="item-title"><strong>{{ item.name }}</strong><span class="ver-badge">v{{ item.latest }}</span><span v-if="installedOf(item.id)" class="installed-badge">已安装</span></div>
              <div class="item-subtitle"><span>{{ item.author || '作者未填写' }}</span></div>
              <p class="item-desc">{{ item.description || '暂无描述' }}</p>
            </div>
          </div>
          <div class="item-details">
            <div class="item-badges">
              <span v-if="item.verified" class="chip verified">已审核</span><span v-else class="chip warn">未审核</span>
              <span v-if="isCurrent(item.id)" class="chip accent">正在使用</span>
              <span v-if="installedOf(item.id)" class="chip">已安装</span>
              <span v-if="installedOf(item.id)?.updateVersion" class="chip warn">可更新至 v{{ installedOf(item.id)?.updateVersion }}</span>
            </div>
            <p v-if="item.repository" class="item-source">来源：{{ item.repository }}</p>
          </div>
          <div class="item-actions">
            <button v-if="installedOf(item.id)?.updateVersion" class="btn accent" :disabled="isInstalling(item.id)" @click="updateCatalogItem(item)">
              <Icon name="refresh" :size="13" :class="{ spin: isInstalling(item.id) }" />{{ isInstalling(item.id) ? '更新中…' : '更新' }}
            </button>
            <button v-else-if="installedOf(item.id)" class="btn apply" :disabled="isCurrent(item.id)" @click="applyCatalogItem(item)">
              <Icon name="info" :size="13" />{{ isCurrent(item.id) ? '使用中' : '详情' }}
            </button>
            <button v-else class="btn accent" :disabled="isInstalling(item.id)" @click="install(item)">
              <Icon :name="isInstalling(item.id) ? 'refresh' : 'plus'" :size="13" :class="{ spin: isInstalling(item.id) }" />{{ isInstalling(item.id) ? '安装中…' : '安装' }}
            </button>
          </div>
        </article>
      </div>
      <div v-else class="results" :class="[viewMode, activeTab]">
        <article v-for="item in visibleInstalled" :key="item.id" class="market-item market-card" tabindex="0" @click="openInstalledDetails(item)" @keydown.enter="openInstalledDetails(item)">
          <div class="item-head">
            <div class="product-icon"><Icon name="palette" :size="21" /></div>
            <div class="item-main">
              <div class="item-title"><strong>{{ item.name }}</strong><span class="ver-badge">v{{ item.version }}</span><span class="installed-badge">已安装</span>
                <span v-if="isCurrent(item.id)" class="chip accent">正在使用</span><span v-if="item.updateVersion" class="chip warn">有更新</span>
              </div>
              <div class="item-subtitle"><span>{{ item.author || '作者未填写' }}</span></div>
              <p class="item-desc">{{ item.description || '暂无描述' }}</p>
            </div>
          </div>
          <div class="item-actions">
            <button class="btn apply" :disabled="isCurrent(item.id)" @click="applySkin(item)"><Icon name="check" :size="13" />{{ isCurrent(item.id) ? '使用中' : '应用' }}</button>
            <button v-if="item.updateVersion" class="btn accent" :disabled="isInstalling(item.id)" :title="'更新到 ' + item.updateVersion" @click="updateInstalled(item)">
              <Icon name="refresh" :size="13" :class="{ spin: isInstalling(item.id) }" />{{ isInstalling(item.id) ? '更新中…' : '更新' }}
            </button>
            <button class="btn danger" :class="{ confirming: confirmRemove[item.id] }" @click="removeInstalled(item)"><Icon name="trash" :size="13" />{{ confirmRemove[item.id] ? '确认移除' : '移除' }}</button>
          </div>
        </article>
      </div>
      <div v-if="visibleCount === 0" class="empty-state">
        <div class="empty-icon"><Icon name="palette" :size="22" /></div>
        <strong>{{ searchQuery ? '没有找到匹配的皮肤' : activeTab === 'updates' ? '暂时没有可更新的皮肤' : activeTab === 'installed' ? '还没有安装皮肤' : '市场目录中暂无皮肤' }}</strong>
        <p>{{ searchQuery ? '试试其他关键词。' : '安装后的皮肤会显示在这里。' }}</p>
        <button v-if="searchQuery" class="btn" @click="searchQuery = ''">清除搜索</button>
      </div>
    </section>
    </template>
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
.market-card { cursor: pointer; transition: transform .16s ease, border-color .16s ease, background .16s ease; }
.market-card:hover, .market-card:focus-visible { border-color: color-mix(in srgb, var(--accent) 55%, var(--border)); background: color-mix(in srgb, var(--accent) 4%, var(--surface-subtle)); transform: translateY(-1px); outline: none; }
.market-card .item-id { display: none; }
.results.discover .item-details { display: none; }
.results.installed .item-actions, .results.updates .item-actions { display: none; }
.results.installed .market-card::after, .results.updates .market-card::after { content: '›'; align-self: flex-end; margin: auto 2px 0 12px; color: var(--text-muted); font-size: 20px; line-height: 1; }
.results.grid .product-icon { width: 36px; height: 36px; border-radius: 10px; }
.results.grid .item-head { gap: 9px; }
.results.grid .item-desc { margin-top: 5px; -webkit-line-clamp: 2; }

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
.btn.apply {
  border-color: transparent;
  background: color-mix(in srgb, var(--accent) 14%, transparent);
  color: var(--accent);
  font-weight: 600;
}
.btn.apply:hover:not(:disabled) {
  border-color: var(--accent);
  color: var(--accent);
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

.installed-badge { display: inline-flex; align-items: center; padding: 3px 7px; border-radius: 999px; background: color-mix(in srgb, var(--accent) 14%, transparent); color: var(--accent); font-size: 10px; font-weight: 650; white-space: nowrap; }

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
.market { gap: 12px; min-height: 0; color: var(--text); }
.market-heading, .market-summary, .heading-actions, .market-controls, .control-actions, .market-tabs,
.item-title, .item-subtitle, .item-badges, .item-actions { display: flex; align-items: center; }
.market-heading { flex: none; justify-content: space-between; gap: 12px; }
.market-summary, .heading-actions, .control-actions { gap: 7px; flex-wrap: wrap; }
.market-summary { min-width: 0; }
.chip { display: inline-flex; align-items: center; padding: 4px 9px; border: 1px solid color-mix(in srgb, var(--border) 70%, transparent); border-radius: 999px; background: var(--surface-subtle); color: var(--text-muted); font-size: 10.5px; font-weight: 600; white-space: nowrap; }
.chip.accent { border-color: transparent; background: color-mix(in srgb, var(--accent) 14%, transparent); color: var(--accent); }
.chip.warn { border-color: transparent; background: color-mix(in srgb, var(--warning) 16%, transparent); color: var(--warning); }
.chip.verified { border-color: color-mix(in srgb, var(--accent) 30%, transparent); color: var(--accent); }
.updated-time { font-weight: 500; }
.heading-actions { flex: none; }
.market-controls { flex: none; gap: 9px; }
.search-field { display: flex; flex: 1 1 220px; align-items: center; gap: 8px; min-width: 160px; height: 36px; padding: 0 10px; border: 1px solid var(--border); border-radius: 10px; background: var(--surface); color: var(--text-muted); }
.search-field:focus-within { border-color: color-mix(in srgb, var(--accent) 70%, var(--border)); box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 12%, transparent); }
.search-field input { width: 100%; min-width: 0; border: 0; outline: 0; background: transparent; color: var(--text); font: inherit; font-size: 12px; }
.search-field input::placeholder { color: var(--text-muted); opacity: .8; }
.clear-search { display: grid; flex: none; place-items: center; width: 23px; height: 23px; border: 0; border-radius: 6px; background: transparent; color: var(--text-muted); }
.clear-search:hover { background: var(--surface-subtle); color: var(--text); }
.control-actions { flex: none; }
.select-control { height: 36px; max-width: 145px; padding: 0 26px 0 10px; border: 1px solid var(--border); border-radius: 9px; background: var(--surface); color: var(--text); font: inherit; font-size: 11px; }
.sort-control { max-width: 130px; }
.view-toggle { display: inline-flex; padding: 3px; border: 1px solid var(--border); border-radius: 9px; background: var(--surface); }
.view-button { display: grid; place-items: center; width: 29px; height: 27px; border: 0; border-radius: 6px; background: transparent; color: var(--text-muted); }
.view-button:hover { color: var(--text); }
.view-button.active { background: color-mix(in srgb, var(--accent) 15%, transparent); color: var(--accent); }
.market-tabs { flex: none; gap: 6px; border-bottom: 1px solid var(--border); }
.market-tab { display: inline-flex; align-items: center; gap: 7px; padding: 8px 10px 9px; margin-bottom: -1px; border: 0; border-bottom: 2px solid transparent; background: transparent; color: var(--text-muted); font-size: 12px; font-weight: 600; }
.market-tab:hover { color: var(--text); }
.market-tab.active { border-bottom-color: var(--accent); color: var(--accent); }
.market-tab span { min-width: 19px; padding: 2px 5px; border-radius: 999px; background: var(--surface-subtle); color: var(--text-muted); font-size: 10px; text-align: center; }
.market-tab span.count-warn { background: color-mix(in srgb, var(--warning) 16%, transparent); color: var(--warning); }
.market-content { flex: 1; min-height: 0; overflow: auto; overscroll-behavior: contain; padding: 1px 2px 8px 1px; }
.results-heading { display: flex; align-items: baseline; justify-content: space-between; gap: 10px; margin: 2px 2px 10px; }
.results-heading > div { display: flex; align-items: baseline; gap: 9px; }
.results-heading h3 { margin: 0; font-size: 13px; font-weight: 650; }
.results-heading span, .filter-summary { color: var(--text-muted); font-size: 10.5px; }
.results { display: grid; gap: 9px; }
.results.grid { grid-template-columns: repeat(auto-fill, minmax(min(100%, 310px), 1fr)); }
.results.list { grid-template-columns: minmax(0, 1fr); }
.market-item { display: flex; min-width: 0; align-items: center; justify-content: space-between; gap: 12px; padding: 12px; border: 1px solid color-mix(in srgb, var(--border) 70%, transparent); border-radius: 12px; background: var(--surface-subtle); }
.results.grid .market-item { min-height: 154px; flex-direction: column; align-items: stretch; padding: 12px; }
.item-head { display: flex; flex: 1; align-items: flex-start; gap: 10px; min-width: 0; }
.product-icon { display: grid; flex: none; place-items: center; width: 40px; height: 40px; border: 1px solid color-mix(in srgb, var(--accent) 18%, transparent); border-radius: 11px; background: color-mix(in srgb, var(--accent) 9%, var(--surface)); color: var(--accent); }
.item-main { min-width: 0; flex: 1; }
.item-title { gap: 7px; flex-wrap: wrap; font-size: 13px; }
.item-title strong { overflow-wrap: anywhere; }
.item-subtitle { flex-wrap: wrap; gap: 5px; margin-top: 3px; color: var(--text-muted); font-size: 10px; }
.item-id { opacity: .82; }
.item-desc { display: -webkit-box; overflow: hidden; margin: 6px 0 0; color: var(--text-muted); font-size: 11px; line-height: 1.5; overflow-wrap: anywhere; -webkit-box-orient: vertical; -webkit-line-clamp: 3; }
.item-details { min-width: 0; }
.item-badges { flex-wrap: wrap; gap: 5px; margin-top: 2px; }
.item-source { overflow: hidden; margin: 7px 0 0; color: var(--text-muted); font-size: 9.5px; text-overflow: ellipsis; white-space: nowrap; }
.ver-badge { padding: 2px 7px; border-radius: 999px; background: color-mix(in srgb, var(--accent) 13%, transparent); color: var(--accent); font-size: 9.5px; font-weight: 650; white-space: nowrap; }
.item-actions { flex: none; justify-content: flex-end; gap: 6px; flex-wrap: wrap; }
.results.grid .item-actions { margin-top: auto; }
.btn.apply { border-color: transparent; background: color-mix(in srgb, var(--accent) 14%, transparent); color: var(--accent); font-weight: 600; }
.btn.apply:hover:not(:disabled) { border-color: var(--accent); color: var(--accent); }
.empty-state, .state { display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 7px; text-align: center; color: var(--text-muted); }
.empty-state { min-height: 180px; margin-top: 6px; padding: 18px; border: 1px dashed color-mix(in srgb, var(--border) 70%, transparent); border-radius: 12px; }
.empty-icon { display: grid; place-items: center; width: 42px; height: 42px; margin-bottom: 3px; border-radius: 13px; background: var(--surface-subtle); color: var(--text-muted); }
.empty-state strong { color: var(--text); font-size: 12px; }
.empty-state p { margin: 0; font-size: 11px; }
.state { flex: 1; }
.state-title { margin: 4px 0 0; color: var(--text); font-size: 13.5px; font-weight: 600; }
.state-desc { max-width: 440px; margin: 0 0 8px; font-size: 12px; line-height: 1.6; }
.spin { animation: spin .9s linear infinite; }
@keyframes spin { to { transform: rotate(360deg); } }
@media (max-width: 720px) {
  .market-heading { align-items: flex-start; flex-direction: column; }
  .market-controls { align-items: stretch; flex-direction: column; }
  .control-actions { justify-content: space-between; }
  .select-control, .sort-control { flex: 1; max-width: none; }
  .results.list .market-item { align-items: flex-start; flex-direction: column; }
  .results.list .item-actions { width: 100%; }
}
@media (max-width: 420px) {
  .updated-time { display: none; }
  .heading-actions { width: 100%; }
  .heading-actions .btn { flex: 1; }
  .results-heading > div { align-items: flex-start; flex-direction: column; gap: 2px; }
}
</style>
