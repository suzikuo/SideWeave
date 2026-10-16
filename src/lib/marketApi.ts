/** GitHub 静态插件/皮肤市场与统一插件宿主命令封装。 */
import { invoke } from '@tauri-apps/api/core'

export interface MarketVersion {
  version: string
  key_id: string
  release: {
    tag: string
    asset: string
    url: string
    signature_url: string
    checksum_url: string
  }
}

/** 市场条目由显式 latest 指针选择版本；客户端不会推算最高 SemVer。 */
export interface MarketItem {
  id: string
  name: string
  icon?: string
  description: string
  author: string
  publisher: string
  type: 'web' | 'native' | 'skin'
  repository: string
  verified: boolean
  latest: string
  capabilities: string[]
  versions: MarketVersion[]
}

/** 市场索引（market_refresh 返回） */
export interface MarketIndex {
  schema_version: number
  /** 更新时间：后端当前序列化为 updated_at，两种键都保留做兼容读取 */
  updatedAt?: string
  updated_at?: string
  plugins: MarketItem[]
  skins: MarketItem[]
}

/** 读取索引更新时间（兼容 updatedAt / updated_at 两种序列化形式） */
export function marketUpdatedAt(index: MarketIndex | null): string {
  return index?.updatedAt ?? index?.updated_at ?? ''
}

export interface MarketInstallInfo {
  id: string
  name: string
  version: string
  publisher: string
  publisherName: string
  packageType: 'web' | 'native' | 'skin'
  verified: boolean
  native: boolean
  trustRequired: boolean
  keyId: string
  keyFingerprint: string
  capabilities: string[]
}

/** 市场内容种类 */
export type MarketKind = 'plugin' | 'skin'

/** 已安装条目（Rust InstalledItem，camelCase 序列化；updateVersion 非空 = 云端有更新）。 */
export interface InstalledItem {
  id: string
  name: string
  version: string
  description: string
  author: string
  kind: MarketKind
  runtime: PluginRuntime | null
  updateVersion: string | null
  appearanceDefaults?: SkinAppearanceDefaults | null
}

/** Skin manifest recommendations used by the opacity reset action. */
export interface SkinAppearanceDefaults {
  sidebarPanelOpacity?: number
  detailPanelOpacity?: number
  controlPanelOpacity?: number
  previewPanelOpacity?: number
  previewImageOpacity?: number
  sidebarImageOpacity?: number
  detailImageOpacity?: number
  componentOpacity?: number
}

/** 已安装清单（market_installed 返回） */
export interface InstalledLists {
  plugins: InstalledItem[]
  skins: InstalledItem[]
}

/** Full-window UI entry points supplied by a skin package. */
export interface SkinUi {
  assetRoot: string
  sidebar: string
  detail: string
  control: string
  preview: string
}

/** 插件自有详情页 UI 元数据（Rust plugin-api::PluginUi） */
export interface PluginUi {
  entry: string
  kind?: 'html' | 'vue' | 'react'
  assetRoot: string
  /** Use the plugin's own detail layout and hide the host identity header. */
  chrome?: 'host' | 'plugin'
  initialWidth?: number
  initialHeight?: number
  minWidth?: number
  minHeight?: number
}

export type PluginRuntime = 'in_process' | 'dynamic_library' | 'external_process' | 'web_sandbox'
export type PluginTrust = 'sandboxed' | 'trusted_local'

export interface RuntimePluginManifest {
  manifestVersion: number
  apiVersion: number
  id: string
  name: string
  icon: string
  version: string
  description: string
  author: string
  runtime: PluginRuntime
  backend: string | null
  trust: PluginTrust
  capabilities: string[]
  ui: PluginUi | null
  minimumHostVersion: string | null
}

export interface RuntimePluginInfo {
  manifest: RuntimePluginManifest
  status: 'discovered' | 'loaded' | 'active' | 'disabled' | 'failed' | 'stopped'
  runtime: PluginRuntime
  location: string
  source: string
  effectiveTrust: PluginTrust
  lastError: { code: string; message: string } | null
}

// ---------- 市场 ----------

/** 读取最近一次校验通过的本地市场缓存，不发起网络请求。 */
export const marketSnapshot = () => invoke<MarketIndex>('market_snapshot')

/** 后台刷新 marketplace.json 和 publishers.json，并使用 ETag 条件请求。 */
export const marketRefresh = () => invoke<MarketIndex>('market_refresh')

export const marketInstalled = () => invoke<InstalledLists>('market_installed')

/** 下载并安装（耗时操作） */
export const marketInstallInfo = (kind: MarketKind, id: string) =>
  invoke<MarketInstallInfo>('market_install_info', { kind, id })

export async function marketInstall(
  kind: MarketKind,
  id: string,
  trustApproval?: Pick<MarketInstallInfo, 'publisher' | 'keyId' | 'keyFingerprint'>
): Promise<void> {
  await invoke<void>('market_install', { kind, id, trustApproval: trustApproval ?? null })
  invalidatePluginRegistry()
}

export async function marketUninstall(kind: MarketKind, id: string): Promise<void> {
  await invoke<void>('market_uninstall', { kind, id })
  invalidatePluginRegistry()
}

/** 本地离线安装：弹出文件框选 zip，不经过市场（便于私有插件分发） */
export async function pluginImportLocal(kind: MarketKind): Promise<void> {
  await invoke<void>('plugin_import_local', { kind })
  invalidatePluginRegistry()
}

// ---------- 用户插件运行时 ----------

let pluginRegistryRequest: Promise<RuntimePluginInfo[]> | null = null
let pluginRegistryCache: RuntimePluginInfo[] | null = null

export function pluginRegistryList(force = false): Promise<RuntimePluginInfo[]> {
  if (force) {
    return invoke<RuntimePluginInfo[]>('plugin_registry_list').then((items) => {
      pluginRegistryCache = items
      return items
    })
  }
  if (pluginRegistryCache) return Promise.resolve(pluginRegistryCache)
  if (!pluginRegistryRequest) {
    pluginRegistryRequest = invoke<RuntimePluginInfo[]>('plugin_registry_list').then((items) => {
      pluginRegistryCache = items
      return items
    }).finally(() => {
      pluginRegistryRequest = null
    })
  }
  return pluginRegistryRequest
}

export function invalidatePluginRegistry(): void {
  pluginRegistryCache = null
}

// ---------- 用户皮肤 ----------

/** 读取用户皮肤 CSS（skin.css） */
export const skinUserCss = (id: string) => invoke<string>('skin_user_css', { id })

/** 读取皮肤的完整 UI 声明；旧式 CSS 皮肤返回 null。 */
export const skinUserUi = (id: string) => invoke<SkinUi | null>('skin_user_ui', { id })
