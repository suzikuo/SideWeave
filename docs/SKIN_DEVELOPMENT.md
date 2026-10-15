# 完整 UI 皮肤开发

完整 UI 皮肤可以接管侧边栏、详情页、控制中心和悬浮预览。每个界面都是独立 HTML 入口，可附带自己的 CSS、JavaScript、字体和图片。只有 `skin.css` 的旧皮肤仍然兼容。

## 创建皮肤包

宿主只内置默认皮肤。官方皮肤（包括像素皮肤）位于 `SideWeave-Plugins/skins/<id>`，并按独立第三方包的 manifest、ZIP、签名和 Marketplace 规范发布。第三方开发者从 `suzikuo/SideWeave-plugin-template/skin` 开始，复制到自己的仓库，再编辑清单、样式和资源。

生成的 `manifest.json` 包含四个入口：

```json
{
  "manifestVersion": 1,
  "apiVersion": 1,
  "id": "my-skin",
  "name": "我的皮肤",
  "version": "1.0.0",
  "type": "skin",
  "description": "我的 SideWeave 皮肤",
  "author": "作者名称",
  "publisher": "your-publisher-id",
  "key_id": "publisher-key-id",
  "requires_trust": false,
  "capabilities": [],
  "skinUi": {
    "assetRoot": "ui",
    "sidebar": "sidebar.html",
    "detail": "detail.html",
    "control": "control.html",
    "preview": "preview.html"
  }
}
```

`assetRoot` 和入口路径都相对于皮肤包。四个入口必须存在于 `assetRoot` 内。HTML 中可像普通网页一样引用相对资源，例如 `./images/background.webp` 和 `./styles/main.css`。打包时会把目录内的文件一并放入皮肤 zip。

`skin.css` 可选。它用于调整仍由宿主绘制的内容，例如插件详情宿主外框；其中相对 `url(...)` 资源以皮肤包根目录为基准。

宿主绘制的桌面通知和侧边栏气泡也会加载当前主题、皮肤令牌及旧式 `skin.css`。默认皮肤定义了 `--notification-*` 令牌；用户皮肤默认从自身的颜色、圆角和阴影令牌生成通知外观，也可以覆盖 `--notification-info/success/warning/alarm`、`--notification-desktop-background/color/radius/shadow/backdrop-filter`、`--notification-bubble-background/color/border/radius/shadow` 等变量，或直接按 `.notification`、`.notification.sidebar-bubble`、`.notification .copy` 等类名改布局和外观。例如：

```css
:root[data-skin='user-my-skin'] {
  --notification-desktop-background: linear-gradient(145deg, #243047, #111827);
  --notification-desktop-color: #f5f7ff;
  --notification-desktop-radius: 18px;
  --notification-bubble-background: #eaf4ff;
  --notification-bubble-color: #17283d;
  --notification-bubble-border: #17283d;
  --notification-bubble-radius: 20px;
}

[data-skin='user-my-skin'] .notification.sidebar-bubble {
  padding: 12px 16px;
}
```

## 宿主 API

宿主会在每个 HTML 入口中注入 `window.sideweaveSkin`。页面运行在隔离 iframe 中，不能直接调用 Tauri。通过 `onContext` 获取当前界面状态，通过 `onEvent` 监听宿主变化，通过 `call` 请求宿主操作。旧的 `window.agileTilesSkin` 名称仍作为兼容别名提供：

```js
const ui = window.sideweaveSkin

ui.onContext((context) => {
  document.body.dataset.surface = context.surface
  // 根据 context.settings、context.plugins、context.page 渲染界面
})

ui.onEvent('settingsChanged', (settings) => {
  // 更新界面中的设置状态
})

await ui.call('navigation.openPlugin', { pluginId: 'bookmarks' })
```

初始上下文包含：

```ts
{
  skinId: string
  surface: 'sidebar' | 'detail' | 'control' | 'preview'
  settings: Settings
  plugins: PluginMeta[] // sidebar context includes enabled, sidebar-visible plugins
  theme: {
    mode: string
    skin: string
    colorScheme: string
    variables: Record<string, string>
  }
  page?: string
  expanded?: boolean
  preview?: PreviewPayload
  appVersion?: string
  dataDir?: string
}
```

宿主会在设置、窗口界面或主题变化时更新上下文。`onEvent` 返回取消订阅函数；支持的事件为 `settingsChanged`、`sidebarExpanded`、`mainSurface` 和 `themeChanged`。

### 可调用操作

| 类别 | 操作 |
|---|---|
| 状态 | `state.get` |
| 设置与插件 | `settings.set`、`plugins.setEnabled`、`plugins.setSidebarVisible`、`plugins.setOrder`、`plugins.list`、`plugins.call` |
| 导航 | `navigation.openDetail`、`navigation.openSettings`、`navigation.openPlugin`、`navigation.openControlCenter` |
| 宿主界面 | `host.showNativeUi`（控制中心内显示完整原生控制项） |
| 窗口 | `window.closeDetail`、`window.minimizeDetail`、`window.toggleSidebar`、`window.startDragging`、`window.startResizeDragging` |
| 侧边栏与预览 | `sidebar.hoverEnter`、`sidebar.collapseIfUnhovered`、`sidebar.previewEnter`、`sidebar.previewLeave`、`sidebar.previewHide`、`preview.getCurrent`、`preview.refresh` |
| 插件市场 | `market.refresh`、`market.installed`、`market.install`、`market.uninstall`、`market.importLocal` |
| 应用服务 | `system.getInfo`、`performance.getStatus`、`update.check`、`update.install`、`update.ignore`、`backup.export`、`backup.import`、`app.openDataDir`、`app.openExternal`、`app.restart` |

设置页、插件市场、皮肤市场和插件详情会保留完整的原生功能。皮肤可以从 `detail` 页面调用相应导航动作进入这些页面；控制中心也可以提供 `host.showNativeUi` 入口，临时切换到完整原生控制中心。皮肤 iframe 中标记 `data-tauri-drag-region` 的区域会转发窗口拖动，`detail` 与 `control` 表面会自动获得边缘缩放手柄。

宿主会在皮肤 iframe 根元素提供三个独立变量：`--skin-panel-opacity`（当前侧边栏、详情页或控制中心的面板底色）、`--skin-image-opacity`（当前表面的皮肤背景图）和 `--skin-component-opacity`（卡片/控件底色），范围均为 `0` 到 `1`。文字和图标不应跟随这些变量变淡。旧变量 `--skin-background-opacity` 仍保留为图片透明度的别名。

操作参数是普通对象，例如：

```js
await ui.call('settings.set', {
  category: 'appearance',
  key: 'theme_mode',
  value: 'dark'
})
await ui.call('window.startResizeDragging', { direction: 'SouthEast' })
await ui.call('market.install', { kind: 'skin', id: 'nebula-console' })
```

## 皮肤推荐透明度

皮肤包可以在 `manifest.json` 的 `appearanceDefaults` 中提供推荐值。设置页和控制中心的“重置透明度”会优先采用当前皮肤声明的值；未声明的字段使用宿主默认值。数值范围为 `0` 到 `1`。

```json
{
  "appearanceDefaults": {
    "sidebarPanelOpacity": 1.0,
    "detailPanelOpacity": 1.0,
    "controlPanelOpacity": 1.0,
    "sidebarImageOpacity": 1.0,
    "detailImageOpacity": 1.0,
    "componentOpacity": 1.0
  }
}
```

只需声明希望覆盖的值。其余字段会沿用宿主默认值；背景图片、面板和组件透明度可以按皮肤风格分别选择。

详情页打开插件时，宿主继续使用现有插件沙箱承载插件内容，保证插件隔离和生命周期正常。

## 本地打包安装

在 SideWeave-Plugins 仓库根目录运行 `tools/build-skin-packages.ps1 -SkinId my-skin`，生成的 ZIP 会把 `manifest.json` 放在压缩包根目录。也可以在 SideWeave 插件/皮肤中心选择“本地导入”，安装后在皮肤中心应用。每次修改入口或资源后重新打包并导入即可查看效果。
