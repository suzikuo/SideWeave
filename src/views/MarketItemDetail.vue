<script setup lang="ts">
import { computed } from 'vue'
import Icon from '../ui/Icon.vue'
import type { InstalledItem, MarketItem } from '../lib/marketApi'
import { openExternalUrl } from '../lib/api'

const props = defineProps<{
  item: MarketItem
  kind: 'plugin' | 'skin'
  installed?: InstalledItem
  busy?: boolean
  removeConfirm?: boolean
  current?: boolean
}>()

const emit = defineEmits<{
  back: []
  install: []
  open: []
  update: []
  apply: []
  remove: []
  'open-repository-error': [message: string]
}>()

async function openRepository() {
  try {
    const url = new URL(props.item.repository)
    if (!['http:', 'https:'].includes(url.protocol) || !url.hostname || url.username || url.password) {
      throw new Error('项目地址不是有效的 HTTP(S) 链接')
    }
    await openExternalUrl(url.href)
  } catch (error) {
    emit('open-repository-error', error instanceof Error ? error.message : String(error))
  }
}

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

const appearanceRows = computed(() => Object.entries(props.installed?.appearanceDefaults ?? {}))
const hasMarketMetadata = computed(() => Boolean(props.item.publisher || props.item.repository || props.item.versions.length))
</script>

<template>
  <section class="market-detail">
    <div class="detail-toolbar">
      <button class="back-button" type="button" @click="emit('back')"><Icon name="chevron-left" :size="16" />返回{{ kind === 'plugin' ? '插件' : '皮肤' }}中心</button>
      <span class="detail-kicker">{{ kind === 'plugin' ? '插件' : '皮肤' }}详情</span>
    </div>

    <div class="detail-scroll">
      <header class="detail-hero">
        <div class="product-icon detail-icon"><Icon :name="kind === 'plugin' ? 'blocks' : 'palette'" :size="24" /></div>
        <div class="detail-heading">
          <div class="detail-title"><h3>{{ item.name }}</h3><span class="ver-badge">v{{ item.latest }}</span></div>
          <div class="detail-subtitle"><span>{{ item.author || '作者未填写' }}</span><span class="detail-separator">·</span><span>{{ item.id }}</span></div>
          <p class="detail-description">{{ item.description || `暂无${kind === 'plugin' ? '插件' : '皮肤'}介绍。` }}</p>
          <div class="detail-badges">
            <span v-if="kind === 'plugin'" class="chip" :class="{ warn: item.type === 'native' }">{{ item.type === 'native' ? 'Native 插件' : 'Web 插件' }}</span>
            <span v-if="hasMarketMetadata" class="chip" :class="item.verified ? 'verified' : 'warn'">{{ item.verified ? '已审核' : '未审核' }}</span>
            <span v-if="installed" class="chip accent">已安装 v{{ installed.version }}</span>
            <span v-if="installed?.updateVersion" class="chip warn">可更新至 v{{ installed.updateVersion }}</span>
            <span v-if="kind === 'skin' && current" class="chip accent">正在使用</span>
          </div>
        </div>
      </header>

      <div class="detail-grid">
        <section class="detail-panel">
          <h4>发布信息</h4>
          <dl class="detail-facts">
            <div><dt>作者</dt><dd>{{ item.author || '未填写' }}</dd></div>
            <div><dt>发布者</dt><dd>{{ item.publisher || '未填写' }}</dd></div>
            <div><dt>最新版本</dt><dd>v{{ item.latest }}</dd></div>
            <div v-if="kind === 'plugin'"><dt>插件类型</dt><dd>{{ item.type === 'native' ? 'Native' : 'Web' }}</dd></div>
            <div v-if="hasMarketMetadata"><dt>审核状态</dt><dd>{{ item.verified ? '已审核' : '未审核' }}</dd></div>
            <div v-if="item.repository"><dt>项目地址</dt><dd><button class="repository-link" type="button" @click="openRepository">{{ item.repository }}</button></dd></div>
          </dl>
        </section>

        <section v-if="kind === 'plugin'" class="detail-panel">
          <h4>请求的宿主权限</h4>
          <div v-if="item.capabilities.length" class="permission-list">
            <div v-for="permission in item.capabilities" :key="permission" class="permission-detail">
              <Icon name="check" :size="13" />
              <span>{{ permissionLabels[permission] ?? permission }}</span>
              <code>{{ permission }}</code>
            </div>
          </div>
          <p v-else-if="hasMarketMetadata" class="detail-empty">未声明宿主 API 权限。</p>
          <p v-else class="detail-empty">当前市场目录没有此插件的权限信息。</p>
          <p v-if="item.type === 'native'" class="native-note">Native 插件代码可直接访问本机资源；权限声明用于说明用途，不构成系统级沙箱。</p>
        </section>

        <section v-else-if="appearanceRows.length" class="detail-panel">
          <h4>皮肤建议</h4>
          <dl class="detail-facts">
            <div v-for="[key, value] in appearanceRows" :key="key"><dt>{{ key }}</dt><dd>{{ value }}%</dd></div>
          </dl>
        </section>

        <section v-if="item.versions.length" class="detail-panel versions-panel">
          <h4>可用版本</h4>
          <div v-for="version in item.versions" :key="version.version" class="version-row">
            <span>v{{ version.version }}</span><span>{{ version.release.tag }}</span>
            <span v-if="version.version === item.latest" class="chip accent">最新</span>
          </div>
        </section>
      </div>

      <footer class="detail-actions">
        <template v-if="kind === 'plugin'">
          <button v-if="installed" class="btn" type="button" @click="emit('open')"><Icon name="play" :size="14" />打开插件</button>
          <button v-if="installed" class="btn danger" :class="{ confirming: removeConfirm }" type="button" @click="emit('remove')"><Icon name="trash" :size="14" />{{ removeConfirm ? '确认移除' : '移除插件' }}</button>
          <button v-if="installed?.updateVersion" class="btn update" type="button" :disabled="busy" @click="emit('update')"><Icon name="refresh" :size="14" :class="{ spin: busy }" />更新到 v{{ installed.updateVersion }}</button>
          <button v-else-if="!installed" class="btn accent" type="button" :disabled="busy" @click="emit('install')"><Icon :name="busy ? 'refresh' : 'plus'" :size="14" :class="{ spin: busy }" />{{ busy ? '正在安装…' : '安装插件' }}</button>
        </template>
        <template v-else>
          <button v-if="installed" class="btn apply" type="button" :disabled="current" @click="emit('apply')"><Icon name="check" :size="14" />{{ current ? '正在使用' : '应用皮肤' }}</button>
          <button v-if="installed" class="btn danger" :class="{ confirming: removeConfirm }" type="button" @click="emit('remove')"><Icon name="trash" :size="14" />{{ removeConfirm ? '确认移除' : '移除皮肤' }}</button>
          <button v-if="installed?.updateVersion" class="btn update" type="button" :disabled="busy" @click="emit('update')"><Icon name="refresh" :size="14" :class="{ spin: busy }" />更新到 v{{ installed.updateVersion }}</button>
          <button v-else-if="!installed" class="btn accent" type="button" :disabled="busy" @click="emit('install')"><Icon :name="busy ? 'refresh' : 'plus'" :size="14" :class="{ spin: busy }" />{{ busy ? '正在安装…' : '安装皮肤' }}</button>
        </template>
      </footer>
    </div>
  </section>
</template>

<style scoped>
.market-detail { display: flex; flex: 1; min-height: 0; flex-direction: column; gap: 12px; color: var(--text); }
.detail-toolbar { display: flex; flex: none; align-items: center; justify-content: space-between; }
.back-button { display: inline-flex; align-items: center; gap: 5px; padding: 5px 8px 5px 4px; border: 0; border-radius: 8px; background: transparent; color: var(--text-muted); font: inherit; font-size: 11px; cursor: pointer; }
.back-button:hover { background: var(--surface-subtle); color: var(--text); }
.detail-kicker { color: var(--text-muted); font-size: 11px; font-weight: 600; }
.detail-scroll { flex: 1; min-height: 0; overflow: auto; padding: 2px 3px 10px; }
.detail-hero { display: flex; gap: 14px; padding: 18px; border: 1px solid color-mix(in srgb, var(--accent) 20%, var(--border)); border-radius: 16px; background: linear-gradient(135deg, color-mix(in srgb, var(--accent) 10%, var(--surface-subtle)), var(--surface-subtle) 65%); }
.detail-icon { width: 48px; height: 48px; border-radius: 14px; }
.detail-heading { min-width: 0; flex: 1; }
.detail-title { display: flex; align-items: center; flex-wrap: wrap; gap: 8px; }
.detail-title h3 { margin: 0; font-size: 17px; font-weight: 700; letter-spacing: -.02em; }
.detail-subtitle { display: flex; align-items: center; gap: 7px; margin-top: 4px; color: var(--text-muted); font-size: 10.5px; }
.detail-separator { opacity: .5; }
.detail-description { max-width: 760px; margin: 10px 0 0; color: var(--text-muted); font-size: 12px; line-height: 1.65; overflow-wrap: anywhere; }
.detail-badges { display: flex; flex-wrap: wrap; gap: 6px; margin-top: 12px; }
.detail-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 10px; margin-top: 10px; }
.detail-panel { min-width: 0; padding: 14px; border: 1px solid color-mix(in srgb, var(--border) 75%, transparent); border-radius: 13px; background: var(--surface-subtle); }
.detail-panel h4 { margin: 0 0 11px; font-size: 11.5px; font-weight: 650; }
.detail-facts { display: grid; gap: 9px; margin: 0; }
.detail-facts > div { display: grid; grid-template-columns: 76px minmax(0, 1fr); align-items: start; gap: 8px; font-size: 10.5px; }
.detail-facts dt { color: var(--text-muted); }
.detail-facts dd { min-width: 0; margin: 0; font-weight: 550; overflow-wrap: anywhere; }
.repository-link { max-width: 100%; padding: 0; border: 0; background: transparent; color: var(--accent); font: inherit; font-weight: 550; text-align: left; text-decoration: underline; text-underline-offset: 2px; overflow-wrap: anywhere; cursor: pointer; }
.repository-link:hover { color: color-mix(in srgb, var(--accent) 78%, var(--text)); }
.repository-link:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; border-radius: 2px; }
.permission-list { display: grid; gap: 7px; }
.permission-detail { display: flex; align-items: center; gap: 7px; min-width: 0; color: var(--accent); font-size: 10.5px; }
.permission-detail span { min-width: 0; overflow: hidden; color: var(--text); text-overflow: ellipsis; white-space: nowrap; }
.permission-detail code { max-width: 48%; min-width: 0; overflow: hidden; margin-left: auto; color: var(--text-muted); font: 9px ui-monospace, SFMono-Regular, Consolas, monospace; text-overflow: ellipsis; white-space: nowrap; }
.detail-empty, .native-note { margin: 0; color: var(--text-muted); font-size: 10.5px; line-height: 1.55; }
.native-note { margin-top: 10px; padding-top: 9px; border-top: 1px solid var(--border); }
.versions-panel { grid-column: 1 / -1; }
.version-row { display: grid; grid-template-columns: 90px minmax(0, 1fr) auto; align-items: center; gap: 9px; padding: 7px 0; border-top: 1px solid var(--border); color: var(--text-muted); font-size: 10.5px; }
.version-row:first-of-type { border-top: 0; }
.version-row span:first-child { color: var(--text); font-weight: 600; }
.version-row span:nth-child(2) { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.detail-actions { display: flex; flex: none; justify-content: flex-end; flex-wrap: wrap; gap: 7px; padding: 11px 3px 2px; border-top: 1px solid var(--border); }
.btn { display: inline-flex; align-items: center; justify-content: center; gap: 6px; min-height: 32px; padding: 0 11px; border: 1px solid var(--border); border-radius: 9px; background: var(--surface); color: var(--text); font: inherit; font-size: 11px; font-weight: 550; cursor: pointer; }
.btn:hover:not(:disabled) { border-color: var(--accent); color: var(--accent); }
.btn:disabled { opacity: .55; cursor: default; }
.btn.accent { border-color: transparent; background: var(--accent); color: #fff; font-weight: 650; }
.btn.accent:hover:not(:disabled) { filter: brightness(1.08); color: #fff; }
.btn.update { border-color: color-mix(in srgb, var(--warning) 46%, transparent); background: color-mix(in srgb, var(--warning) 12%, var(--surface)); color: var(--warning); font-weight: 650; }
.btn.update:hover:not(:disabled) { border-color: var(--warning); background: color-mix(in srgb, var(--warning) 19%, var(--surface)); color: var(--warning); }
.btn.apply { border-color: transparent; background: color-mix(in srgb, var(--accent) 14%, transparent); color: var(--accent); }
.btn.danger { color: var(--danger); }
.btn.danger.confirming { border-color: var(--danger); background: var(--danger); color: #fff; }
.spin { animation: spin .9s linear infinite; }
@keyframes spin { to { transform: rotate(360deg); } }
@media (max-width: 640px) { .detail-grid { grid-template-columns: 1fr; } .versions-panel { grid-column: auto; } .detail-hero { padding: 14px; } }
</style>
