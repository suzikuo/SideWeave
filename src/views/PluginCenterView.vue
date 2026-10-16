<script setup lang="ts">
import Button from '@/components/ui/button/Button.vue'
import Input from '@/components/ui/input/Input.vue'
import { TabsList, TabsRoot, TabsTrigger } from 'reka-ui'
/** 插件中心：浏览 SideWeave 市场，安装 / 更新 / 打开 / 移除独立插件。 */
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import Icon from '../ui/Icon.vue'
import Toast from '../ui/Toast.vue'
import MarketItemDetail from './MarketItemDetail.vue'
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

type MarketTab = 'discover' | 'installed' | 'updates'
type MarketView = 'grid' | 'list'
type SortMode = 'directory' | 'name-asc' | 'name-desc'
type PluginTypeFilter = 'all' | 'web' | 'native'

const activeTab = ref<MarketTab>('discover')
const searchQuery = ref('')
const typeFilter = ref<PluginTypeFilter>('all')
const verifiedOnly = ref(false)
const sortMode = ref<SortMode>('directory')
watch(activeTab, (tab) => {
  if (tab !== 'discover') {
    typeFilter.value = 'all'
    verifiedOnly.value = false
  }
})

function readSavedView(): MarketView {
  try {
    return window.localStorage.getItem('sideweave.market.plugin.view') === 'list' ? 'list' : 'grid'
  } catch {
    return 'grid'
  }
}

const viewMode = ref<MarketView>(readSavedView())
watch(viewMode, (value) => {
  try {
    window.localStorage.setItem('sideweave.market.plugin.view', value)
  } catch {
    // The view switch remains usable when browser storage is unavailable.
  }
})

const catalogItems = computed<MarketItem[]>(() => index.value?.plugins ?? [])
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

const visibleCatalog = computed(() => {
  const query = searchQuery.value.trim()
  const items = catalogItems.value.filter((item) => {
    if (typeFilter.value !== 'all' && item.type !== typeFilter.value) return false
    if (verifiedOnly.value && !item.verified) return false
    return matchesSearch(query, [
      item.name, item.id, item.description, item.author, item.publisher,
      item.type, item.repository, item.verified ? '已审核 verified' : '待审核 unverified',
      ...permissionNames(item.capabilities)
    ])
  })
  return sortByName(items)
})

const visibleInstalled = computed(() => {
  const items = installed.value.filter((item) => {
    if (activeTab.value === 'updates' && !item.updateVersion) return false
    return matchesSearch(searchQuery.value, [
      item.name, item.id, item.description, item.author, item.version,
      item.updateVersion ?? '', item.runtime ?? ''
    ])
  })
  return sortByName(items)
})

const visibleCount = computed(() => activeTab.value === 'discover' ? visibleCatalog.value.length : visibleInstalled.value.length)
const tabTitle = computed(() => activeTab.value === 'discover' ? '发现插件' : activeTab.value === 'installed' ? '已安装插件' : '可更新插件')

const importingLocal = ref(false)

const permissionLabels: Record<string, string> = {
  storage: '插件设置存储',
  'notification.system': 'Windows 原生通知',
  'notification.desktop': '宿主桌面通知',
  'notification.sidebar': '侧边栏气泡',
  notification: '系统与桌面通知',
  'event.publish': '发布宿主事件',
  'filesystem.read': '读取插件文件',
  'filesystem.write': '写入插件文件',
  'filesystem.external_read': '读取外部文件',
  clipboard: '剪贴板',
  'process.spawn': '启动外部进程',
  'network.listen': '监听网络端口',
  'network.connect': '连接网络',
  'window.create': '创建原生窗口',
  'window.manage': '控制插件窗口',
  'shortcut.global': '全局快捷键',
  'system.open_external': '打开外部链接',
  'system.file_picker': '文件选择器',
  'system.icon': '提取文件图标'
}

function permissionNames(capabilities: string[]): string[] {
  return [...new Set(capabilities.map((capability) => permissionLabels[capability] ?? capability))]
}

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
    type: item.runtime === 'web_sandbox' ? 'web' : 'native',
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
    showToast(`市场中没有找到插件「${id}」`, 'error')
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
    let trustApproval: { publisher: string; keyId: string; keyFingerprint: string } | undefined
    const capabilities = permissionNames(info.capabilities)
    const permissionSummary = capabilities.length ? capabilities.join('、') : '未声明宿主 API 权限'
    const review = info.verified ? '' : '\n市场维护者尚未审核此条目。'
    const runtimeNote = info.native
      ? '\nNative 代码可绕过宿主 API 直接访问本机资源；这些能力只是声明信息，不构成系统级沙箱。'
      : '\n安装后，宿主 API 会按插件权限设置决定是否放行对应调用。'
    const trust = info.native && info.trustRequired
      ? `\n\n这是首次信任此发布者密钥。\nMinisign 指纹：${info.keyFingerprint}`
      : ''
    const accepted = window.confirm(
      `即将安装「${info.name}」v${info.version}\n发布者：${info.publisherName} (${info.publisher})\n类型：${info.packageType}\n请求权限：${permissionSummary}${review}${runtimeNote}${trust}\n\n继续安装？`
    )
    if (!accepted) return
    if (info.native && info.trustRequired) {
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

function openCatalogItem(item: MarketItem) {
  openDetails(item)
}

/** 打开已安装插件：所有运行时统一交给详情宿主解析。 */
async function openInstalled(it: InstalledItem) {
  try {
    await openPluginDetail(it.id)
  } catch (e) {
    showToast(String(e), 'error')
  }
}

function openSelectedPlugin() {
  const item = selectedItem.value && installedOf(selectedItem.value.id)
  if (item) void openInstalled(item)
}

function removeSelectedPlugin() {
  const item = selectedItem.value && installedOf(selectedItem.value.id)
  if (item) void removeInstalled(item)
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
    <MarketItemDetail
      v-if="selectedItem"
      :item="selectedItem"
      :kind="KIND"
      :installed="installedOf(selectedItem.id)"
      :busy="isInstalling(selectedItem.id)"
      :remove-confirm="confirmRemove[selectedItem.id]"
      @back="selectedItem = null"
      @install="install(selectedItem)"
      @open="openSelectedPlugin"
      @update="updateCatalogItem(selectedItem)"
      @remove="removeSelectedPlugin"
      @open-repository-error="showToast($event, 'error')"
    />
    <template v-else>
    <div class="toolbar">
      <div class="chips">
        <span class="chip accent">目录 {{ catalogItems.length }}</span>
        <span class="chip">已安装 {{ installed.length }}</span>
        <span v-if="updateCount" class="chip warn">待更新 {{ updateCount }}</span>
        <span v-if="updatedAtText" class="chip updated-time" title="市场目录更新时间">目录更新：{{ updatedAtText }}</span>
      </div>
      <div class="btn-group">
        <Button variant="outline" size="sm" class="btn" :disabled="loading || importingLocal" @click="importLocal"><Icon name="plus" :size="14" /> 本地导入</Button>
        <Button variant="outline" size="sm" class="btn" :disabled="loading" @click="reload"><Icon name="refresh" :size="14" :class="{ spin: loading }" /> 刷新目录</Button>
      </div>
    </div>

    <section class="market-controls" aria-label="插件市场工具">
      <label class="search-field">
        <Icon name="search" :size="16" />
        <Input v-model="searchQuery" class="market-search-input" type="search" placeholder="搜索名称、描述、发布者或权限…" aria-label="搜索插件" />
        <button v-if="searchQuery" class="clear-search" type="button" aria-label="清除搜索" @click="searchQuery = ''"><Icon name="x" :size="14" /></button>
      </label>
      <div class="control-actions">
        <template v-if="activeTab === 'discover'">
          <select v-model="typeFilter" class="select-control" aria-label="按插件类型筛选">
            <option value="all">所有类型</option><option value="web">Web 插件</option><option value="native">Native Beta</option>
          </select>
          <Button variant="outline" size="sm" class="btn filter-button" :class="{ selected: verifiedOnly }" type="button" :aria-pressed="verifiedOnly" @click="verifiedOnly = !verifiedOnly"><Icon name="check" :size="14" /> 仅看已审核</Button>
        </template>
        <select v-model="sortMode" class="select-control sort-control" aria-label="插件排序">
          <option value="directory">目录顺序</option><option value="name-asc">名称 A–Z</option><option value="name-desc">名称 Z–A</option>
        </select>
        <div class="view-toggle" role="group" aria-label="展示方式">
          <button class="view-button" :class="{ active: viewMode === 'grid' }" type="button" aria-label="卡片视图" title="卡片视图" :aria-pressed="viewMode === 'grid'" @click="viewMode = 'grid'"><Icon name="grid" :size="15" /></button>
          <button class="view-button" :class="{ active: viewMode === 'list' }" type="button" aria-label="列表视图" title="列表视图" :aria-pressed="viewMode === 'list'" @click="viewMode = 'list'"><Icon name="list" :size="16" /></button>
        </div>
      </div>
    </section>

    <TabsRoot v-model="activeTab" class="market-tabs-root">
      <TabsList class="market-tabs" aria-label="插件分类">
        <TabsTrigger value="discover" class="market-tab">发现 <span>{{ catalogItems.length }}</span></TabsTrigger>
        <TabsTrigger value="installed" class="market-tab">已安装 <span>{{ installed.length }}</span></TabsTrigger>
        <TabsTrigger value="updates" class="market-tab">有更新 <span :class="{ 'count-warn': updateCount }">{{ updateCount }}</span></TabsTrigger>
      </TabsList>
    </TabsRoot>

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
      <Button variant="outline" size="sm" class="btn" @click="reload"><Icon name="refresh" :size="14" /> 重试</Button>
    </div>

    <section v-else class="market-content" aria-live="polite">
      <div class="results-heading"><div><h3>{{ tabTitle }}</h3><span>显示 {{ visibleCount }} 项</span></div><span v-if="searchQuery || typeFilter !== 'all' || verifiedOnly" class="filter-summary">筛选结果</span></div>
      <div v-if="activeTab === 'discover'" class="results" :class="[viewMode, activeTab]">
        <article v-for="item in visibleCatalog" :key="item.id" class="market-item market-card" tabindex="0" @click="openDetails(item)" @keydown.enter="openDetails(item)">
          <div class="item-head"><div class="product-icon"><Icon name="blocks" :size="21" /></div><div class="item-main">
            <div class="item-title"><strong>{{ item.name }}</strong><span class="ver-badge">v{{ item.latest }}</span><span v-if="installedOf(item.id)" class="installed-badge">已安装</span></div>
            <div class="item-subtitle"><span>{{ item.author || '作者未填写' }}</span></div>
            <p class="item-desc">{{ item.description || '暂无描述' }}</p>
          </div></div>
          <div class="item-details">
            <div class="item-badges"><span class="chip" :class="{ warn: item.type === 'native' }">{{ item.type === 'native' ? 'Native Beta' : 'Web 插件' }}</span>
              <span v-if="item.verified" class="chip verified">已审核</span><span v-else class="chip warn">未审核</span>
              <span v-if="installedOf(item.id)" class="chip accent">已安装</span><span v-if="installedOf(item.id)?.updateVersion" class="chip warn">可更新至 v{{ installedOf(item.id)?.updateVersion }}</span>
            </div>
            <div class="item-permissions"><strong>请求权限</strong><template v-if="item.capabilities.length"><span v-for="permission in permissionNames(item.capabilities)" :key="permission" class="permission-chip">{{ permission }}</span></template><span v-else class="permission-empty">未声明宿主 API 权限</span></div>
            <p v-if="item.repository" class="item-source">来源：{{ item.repository }}</p>
          </div>
          <div class="item-actions">
            <Button variant="outline" size="sm" v-if="installedOf(item.id)?.updateVersion" class="btn update" :disabled="isInstalling(item.id)" @click="updateCatalogItem(item)"><Icon name="refresh" :size="13" :class="{ spin: isInstalling(item.id) }" />{{ isInstalling(item.id) ? '更新中…' : '更新' }}</Button>
            <Button variant="outline" size="sm" v-else-if="installedOf(item.id)" class="btn" @click="openCatalogItem(item)"><Icon name="info" :size="13" />详情</Button>
            <Button variant="default" size="sm" v-else class="btn accent" :disabled="isInstalling(item.id)" @click="install(item)"><Icon :name="isInstalling(item.id) ? 'refresh' : 'plus'" :size="13" :class="{ spin: isInstalling(item.id) }" />{{ isInstalling(item.id) ? '安装中…' : '安装' }}</Button>
          </div>
        </article>
      </div>
      <div v-else class="results" :class="[viewMode, activeTab]">
        <article v-for="item in visibleInstalled" :key="item.id" class="market-item market-card" tabindex="0" @click="openInstalledDetails(item)" @keydown.enter="openInstalledDetails(item)">
          <div class="item-head"><div class="product-icon"><Icon name="blocks" :size="21" /></div><div class="item-main">
            <div class="item-title"><strong>{{ item.name }}</strong><span class="ver-badge">v{{ item.version }}</span><span class="installed-badge">已安装</span><span v-if="item.runtime === 'dynamic_library'" class="chip">本地 DLL</span><span v-else-if="item.runtime === 'external_process'" class="chip">独立进程</span><span v-if="item.updateVersion" class="chip warn">有更新</span></div>
            <div class="item-subtitle"><span>{{ item.author || '作者未填写' }}</span></div>
            <p class="item-desc">{{ item.description || '暂无描述' }}</p>
          </div></div>
          <div class="item-actions"><Button variant="outline" size="sm" class="btn" @click="openInstalled(item)"><Icon name="play" :size="13" />打开</Button>
            <Button variant="outline" size="sm" v-if="item.updateVersion" class="btn update" :disabled="isInstalling(item.id)" :title="'更新到 ' + item.updateVersion" @click="updateInstalled(item)"><Icon name="refresh" :size="13" :class="{ spin: isInstalling(item.id) }" />{{ isInstalling(item.id) ? '更新中…' : '更新' }}</Button>
            <Button variant="destructive" size="sm" class="btn danger" :class="{ confirming: confirmRemove[item.id] }" @click="removeInstalled(item)"><Icon name="trash" :size="13" />{{ confirmRemove[item.id] ? '确认移除' : '移除' }}</Button>
          </div>
        </article>
      </div>
      <div v-if="visibleCount === 0" class="empty-state"><div class="empty-icon"><Icon name="blocks" :size="22" /></div>
        <strong>{{ searchQuery || typeFilter !== 'all' || verifiedOnly ? '没有找到匹配的插件' : activeTab === 'updates' ? '暂时没有可更新的插件' : activeTab === 'installed' ? '还没有安装插件' : '市场目录中暂无插件' }}</strong>
        <p>{{ searchQuery || typeFilter !== 'all' || verifiedOnly ? '试试其他关键词或清除筛选条件。' : '插件安装后会显示在这里。' }}</p>
        <Button variant="outline" size="sm" v-if="searchQuery || typeFilter !== 'all' || verifiedOnly" class="btn" @click="searchQuery = ''; typeFilter = 'all'; verifiedOnly = false">清除筛选</Button>
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
.market-card:hover, .market-card:focus-visible { border-color: color-mix(in srgb, var(--primary) 55%, var(--border)); background: color-mix(in srgb, var(--primary) 4%, var(--surface-subtle)); transform: translateY(-1px); outline: none; }
.market-card .item-id { display: none; }
.results.discover .item-details { display: none; }
.results.installed .item-actions, .results.updates .item-actions { display: none; }
.results.installed .market-card::after, .results.updates .market-card::after { content: '›'; align-self: flex-end; margin: auto 2px 0 12px; color: var(--text-muted); font-size: 20px; line-height: 1; }
.results.installed .item-title .chip:not(.warn), .results.updates .item-title .chip:not(.warn) { display: none; }
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
  background: color-mix(in srgb, var(--primary) 14%, transparent);
  color: var(--primary);
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
  padding: 0 12px;
  border-radius: 6px;
  border: 1px solid var(--border);
  background: var(--surface);
  color: var(--text);
  font-size: 13px;
  white-space: nowrap;
}
.btn:hover:not(:disabled) {
  border-color: var(--border-strong);
  color: var(--accent-foreground);
}
.btn:disabled {
  opacity: 0.55;
  cursor: default;
}
.btn.accent {
  border-color: transparent;
  background: var(--primary);
  color: var(--primary-foreground);
  font-weight: 600;
}
.btn.accent:hover:not(:disabled) {
  filter: brightness(1.1);
  color: var(--primary-foreground);
}
.btn.update {
  border-color: color-mix(in srgb, var(--warning) 46%, transparent);
  background: color-mix(in srgb, var(--warning) 12%, var(--surface));
  color: var(--warning);
  font-weight: 600;
}
.btn.update:hover:not(:disabled) {
  border-color: var(--warning);
  background: color-mix(in srgb, var(--warning) 19%, var(--surface));
  color: var(--warning);
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

.installed-badge { display: inline-flex; align-items: center; padding: 3px 7px; border-radius: 999px; background: color-mix(in srgb, var(--primary) 14%, transparent); color: var(--primary); font-size: 10px; font-weight: 650; white-space: nowrap; }

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

.item-permissions {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 5px;
  margin-top: 7px;
  font-size: 10.5px;
}
.item-permissions strong {
  color: var(--text-muted);
  font-weight: 600;
  margin-right: 2px;
}
.permission-chip {
  padding: 2px 7px;
  border: 1px solid color-mix(in srgb, var(--warning) 35%, transparent);
  border-radius: 999px;
  color: color-mix(in srgb, var(--warning) 80%, var(--text));
}
.permission-empty {
  color: var(--text-muted);
}

.ver-badge {
  padding: 1.5px 8px;
  border-radius: 999px;
  background: color-mix(in srgb, var(--primary) 13%, transparent);
  color: var(--primary);
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
.toolbar { flex: none; }
.updated-time { font-weight: 500; }
.market-controls, .control-actions, .market-tabs { display: flex; align-items: center; }
.market-controls { flex: none; gap: 9px; }
.control-actions { flex: none; gap: 7px; flex-wrap: wrap; }
.search-field { display: flex; flex: 1 1 220px; align-items: center; gap: 8px; min-width: 160px; height: 36px; padding: 0 10px; border: 1px solid var(--border); border-radius: 10px; background: var(--surface); color: var(--text-muted); }
.search-field:focus-within { border-color: color-mix(in srgb, var(--primary) 70%, var(--border)); box-shadow: 0 0 0 3px color-mix(in srgb, var(--primary) 12%, transparent); }
.search-field input { width: 100%; min-width: 0; border: 0; outline: 0; background: transparent; color: var(--text); font: inherit; font-size: 12px; }
.search-field input::placeholder { color: var(--text-muted); opacity: .8; }
.clear-search { display: grid; flex: none; place-items: center; width: 23px; height: 23px; border: 0; border-radius: 6px; background: transparent; color: var(--text-muted); }
.clear-search:hover { background: var(--surface-subtle); color: var(--text); }
.select-control { height: 36px; max-width: 145px; padding: 0 26px 0 10px; border: 1px solid var(--border); border-radius: 9px; background: var(--surface); color: var(--text); font: inherit; font-size: 11px; }
.sort-control { max-width: 130px; }
.filter-button.selected { border-color: color-mix(in srgb, var(--primary) 50%, var(--border)); background: color-mix(in srgb, var(--primary) 10%, var(--surface)); color: var(--primary); }
.view-toggle { display: inline-flex; padding: 3px; border: 1px solid var(--border); border-radius: 9px; background: var(--surface); }
.view-button { display: grid; place-items: center; width: 29px; height: 27px; border: 0; border-radius: 6px; background: transparent; color: var(--text-muted); }
.view-button:hover { color: var(--text); }
.view-button.active { background: color-mix(in srgb, var(--primary) 15%, transparent); color: var(--primary); }
.market-tabs { flex: none; gap: 6px; border-bottom: 1px solid var(--border); }
.market-tab { display: inline-flex; align-items: center; gap: 7px; padding: 8px 10px 9px; margin-bottom: -1px; border: 0; border-bottom: 2px solid transparent; background: transparent; color: var(--text-muted); font-size: 12px; font-weight: 600; }
.market-tab:hover { color: var(--text); }
.market-tab.active { border-bottom-color: var(--primary); color: var(--primary); }
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
.product-icon { display: grid; flex: none; place-items: center; width: 40px; height: 40px; border: 1px solid color-mix(in srgb, var(--primary) 18%, transparent); border-radius: 11px; background: color-mix(in srgb, var(--primary) 9%, var(--surface)); color: var(--primary); }
.item-subtitle { display: flex; flex-wrap: wrap; gap: 5px; margin-top: 3px; color: var(--text-muted); font-size: 10px; }
.item-id { opacity: .82; }
.item-desc { display: -webkit-box; overflow: hidden; -webkit-box-orient: vertical; -webkit-line-clamp: 3; }
.item-details { min-width: 0; }
.item-badges, .item-permissions { display: flex; align-items: center; flex-wrap: wrap; gap: 5px; }
.item-badges { margin-top: 2px; }
.item-permissions { align-items: flex-start; margin-top: 8px; font-size: 10px; }
.item-permissions strong { margin: 3px 2px 0 0; color: var(--text-muted); font-weight: 600; }
.chip.verified { border-color: color-mix(in srgb, var(--primary) 30%, transparent); color: var(--primary); }
.permission-chip { padding: 3px 7px; border: 1px solid color-mix(in srgb, var(--warning) 34%, transparent); border-radius: 999px; color: color-mix(in srgb, var(--warning) 80%, var(--text)); font-size: 9.5px; }
.permission-empty { margin-top: 3px; color: var(--text-muted); }
.item-source { overflow: hidden; margin: 7px 0 0; color: var(--text-muted); font-size: 9.5px; text-overflow: ellipsis; white-space: nowrap; }
.results.grid .item-actions { margin-top: auto; }
.empty-state { display: flex; min-height: 180px; flex-direction: column; align-items: center; justify-content: center; gap: 7px; margin-top: 6px; padding: 18px; border: 1px dashed color-mix(in srgb, var(--border) 70%, transparent); border-radius: 12px; text-align: center; color: var(--text-muted); }
.empty-icon { display: grid; place-items: center; width: 42px; height: 42px; margin-bottom: 3px; border-radius: 13px; background: var(--surface-subtle); }
.empty-state strong { color: var(--text); font-size: 12px; }
.empty-state p { margin: 0; font-size: 11px; }
@media (max-width: 720px) {
  .market-controls { align-items: stretch; flex-direction: column; }
  .control-actions { justify-content: space-between; }
  .select-control, .sort-control { flex: 1; max-width: none; }
  .results.list .market-item { align-items: flex-start; flex-direction: column; }
  .results.list .item-actions { width: 100%; }
}
@media (max-width: 420px) {
  .updated-time { display: none; }
  .control-actions { gap: 5px; }
  .filter-button { padding-inline: 8px; }
}

.market { gap: 11px; }
.market-tabs-root { display: contents; }
.toolbar { min-height: 34px; padding: 0; border: 0; background: transparent; }
.chips .chip { border: 1px solid var(--border); border-radius: 5px; background: var(--background); color: var(--muted-foreground); font-size: 9px; }
.chips .chip.accent { border-color: color-mix(in srgb, var(--primary) 28%, var(--border)); background: color-mix(in srgb, var(--primary) 7%, var(--background)); color: var(--primary); }
.market-controls { gap: 8px; padding: 0; border: 0; background: transparent; }
.search-field { min-height: 34px; border: 1px solid var(--input); border-radius: 6px; background: var(--background); color: var(--muted-foreground); box-shadow: 0 1px 2px rgb(0 0 0 / 3%); }
.search-field:focus-within { border-color: var(--ring); box-shadow: 0 0 0 2px color-mix(in srgb, var(--ring) 20%, transparent); }
.search-field input { color: var(--foreground); font-size: 10px; }
.search-field :deep(.market-search-input) { height: 32px; padding: 0; border: 0; background: transparent; box-shadow: none; font-size: 10px; }
.search-field :deep(.market-search-input:focus-visible) { outline: 0; box-shadow: none; }
.search-field input::placeholder { color: var(--muted-foreground); }
.select-control { min-height: 34px; border-color: var(--input); border-radius: 6px; background-color: var(--background); color: var(--foreground); font-size: 9px; }
.market :deep(button.btn) { min-height: 31px; border-radius: 6px; font-size: 9px; }
.market :deep(button.btn.accent) { border-color: var(--primary); background: var(--primary); color: var(--primary-foreground); }
.market-tabs { gap: 3px; border-bottom: 1px solid var(--border); }
.market-tab { min-height: 32px; padding: 0 10px; border: 0; border-bottom: 2px solid transparent; border-radius: 0; color: var(--muted-foreground); font-size: 10px; }
.market-tab[data-state='active'] { border-bottom-color: var(--primary); background: transparent; color: var(--foreground); }
.market-tab span { border: 1px solid var(--border); border-radius: 4px; background: var(--muted); color: var(--muted-foreground); font-size: 8px; }
.results-heading { padding: 0 1px; }
.results-heading h3 { font-size: 11px; font-weight: 600; }
.results-heading span, .filter-summary { color: var(--muted-foreground); font-size: 9px; }
.results { gap: 8px; }
.results.grid { grid-template-columns: repeat(auto-fill, minmax(min(100%, 290px), 1fr)); }
.market-item { padding: 12px; border: 1px solid var(--border); border-radius: 8px; background: var(--card); box-shadow: 0 1px 2px rgb(0 0 0 / 3%); transition: border-color .15s, box-shadow .15s, transform .15s; }
.market-item:hover { transform: translateY(-1px); border-color: color-mix(in srgb, var(--primary) 35%, var(--border)); box-shadow: 0 5px 14px rgb(0 0 0 / 7%); }
.results.grid .market-item { min-height: 182px; gap: 9px; }
.product-icon { width: 34px; height: 34px; border: 1px solid var(--border); border-radius: 7px; background: var(--muted); color: var(--foreground); }
.item-title strong { color: var(--foreground); font-size: 11px; font-weight: 600; }
.item-subtitle, .item-desc, .item-source { color: var(--muted-foreground); }
.item-desc { font-size: 9px; line-height: 1.5; }
.chip, .ver-badge, .installed-badge, .permission-chip { padding: 2px 6px; border: 1px solid var(--border); border-radius: 4px; background: var(--background); color: var(--muted-foreground); font-size: 8px; }
.chip.verified, .chip.accent { border-color: color-mix(in srgb, var(--primary) 28%, var(--border)); background: color-mix(in srgb, var(--primary) 7%, var(--background)); color: var(--primary); }
.chip.warn, .permission-chip { border-color: color-mix(in srgb, var(--warning) 28%, var(--border)); background: color-mix(in srgb, var(--warning) 6%, var(--background)); color: var(--warning); }
.item-permissions { padding-top: 8px; border-top: 1px solid var(--border); }
.item-permissions strong { color: var(--muted-foreground); font-size: 8px; }
.item-actions { gap: 5px; }
.empty-state { min-height: 150px; border-color: var(--border); border-radius: 8px; background: var(--background); }
.empty-state strong { color: var(--foreground); font-size: 10px; }
.empty-state p { color: var(--muted-foreground); font-size: 9px; }

/* 插件中心的卡片和控件跟随全局组件底色不透明度。 */
.market-card:hover,
.market-card:focus-visible {
  background: color-mix(
    in srgb,
    color-mix(in srgb, var(--primary) 4%, var(--surface-subtle))
      calc(var(--component-opacity, 1) * 100%),
    transparent
  );
}
.market-card,
.search-field,
.select-control,
.view-toggle,
.market-item,
.product-icon,
.chip,
.ver-badge,
.installed-badge,
.permission-chip,
.empty-icon,
.empty-state {
  backdrop-filter: var(--component-glass-filter, none);
  -webkit-backdrop-filter: var(--component-glass-filter, none);
}
.chips .chip,
.search-field,
.select-control,
.view-toggle {
  background: color-mix(in srgb, var(--background) calc(var(--component-opacity, 1) * 100%), transparent);
}
.chips .chip.accent {
  background: color-mix(
    in srgb,
    color-mix(in srgb, var(--primary) 7%, var(--background))
      calc(var(--component-opacity, 1) * 100%),
    transparent
  );
}
.market :deep(button.btn) {
  background: color-mix(in srgb, var(--surface) calc(var(--component-opacity, 1) * 100%), transparent);
  backdrop-filter: var(--component-glass-filter, none);
  -webkit-backdrop-filter: var(--component-glass-filter, none);
}
.market :deep(button.btn.accent) {
  background: color-mix(in srgb, var(--primary) calc(var(--component-opacity, 1) * 100%), transparent);
}
.market :deep(button.btn.update) {
  background: color-mix(
    in srgb,
    color-mix(in srgb, var(--warning) 12%, var(--surface))
      calc(var(--component-opacity, 1) * 100%),
    transparent
  );
}
.market :deep(button.btn.danger.confirming) {
  background: color-mix(in srgb, var(--destructive) calc(var(--component-opacity, 1) * 100%), transparent);
}
.filter-button.selected {
  background: color-mix(
    in srgb,
    color-mix(in srgb, var(--primary) 10%, var(--surface))
      calc(var(--component-opacity, 1) * 100%),
    transparent
  );
}
.clear-search:hover,
.market-tab span,
.product-icon,
.empty-icon {
  background: color-mix(in srgb, var(--muted) calc(var(--component-opacity, 1) * 100%), transparent);
}
.view-button.active {
  background: color-mix(in srgb, var(--primary) calc(var(--component-opacity, 1) * 15%), transparent);
}
.market-item {
  background: color-mix(in srgb, var(--card) calc(var(--component-opacity, 1) * 100%), transparent);
}
.chip,
.ver-badge,
.installed-badge,
.permission-chip,
.detail-badges .ver-badge {
  background: color-mix(in srgb, var(--background) calc(var(--component-opacity, 1) * 100%), transparent);
}
.chip.verified,
.chip.accent {
  background: color-mix(
    in srgb,
    color-mix(in srgb, var(--primary) 7%, var(--background))
      calc(var(--component-opacity, 1) * 100%),
    transparent
  );
}
.chip.warn,
.permission-chip {
  background: color-mix(
    in srgb,
    color-mix(in srgb, var(--warning) 6%, var(--background))
      calc(var(--component-opacity, 1) * 100%),
    transparent
  );
}
.empty-state {
  background: color-mix(in srgb, var(--background) calc(var(--component-opacity, 1) * 100%), transparent);
}
</style>
