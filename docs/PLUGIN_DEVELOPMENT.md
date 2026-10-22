# SideWeave 插件开发

## 目录约定

插件包由三部分组成：

```text
my-plugin/
├── Cargo.toml
├── manifest.json
├── src/lib.rs
└── ui/
    ├── index.html
    ├── public/icon.svg
    └── src/
```

`manifest.json` 使用 `manifestVersion`、`apiVersion`、`icon`、`runtime`、`trust`、`capabilities` 和 `ui` 字段。旧的 `permissions` 字段继续读取。`icon` 是相对插件包根目录的图标资源路径，示例里的 `ui/public/icon.svg` 会由 Vite 复制为 `ui/icon.svg`，所以 manifest 填 `ui/icon.svg`。支持 SVG、PNG、WebP、JPEG 和 ICO；留空、文件缺失或格式不支持时，宿主显示默认拼图图标。宿主按图标透明度使用当前主题前景色着色，建议提供透明背景的单色线框 SVG。`trust` 只是插件声明，最终有效信任由宿主授予；插件不能靠 manifest 自己获得 `trusted_local`。

## 本地开发

官方插件源码位于旁边的 `SideWeave-Plugins` 仓库。第三方 Web 与 Native 插件模板都在 `suzikuo/SideWeave-plugin-template`：Web 模板位于仓库根目录，独立 Native DLL 模板位于 `native/`。Native starter 自带 Cargo 工程、DLL/资源打包脚本和本地签名发布脚本；官方仓库中的 `plugins/native-template` 仍是官方 workspace 示例。

Rust 插件依赖 `sideweave-plugin-api` 提供协议类型，依赖 `sideweave-plugin-sdk` 提供宿主能力；external-process 插件再依赖 `sideweave-plugin-ipc`。这几个 crate 应使用同一个经审核的 SideWeave Git revision，并提交 `Cargo.lock` 固定该 revision。插件业务代码不要依赖 `src-tauri`、宿主 runtime crate 或 Tauri `AppHandle`。

```powershell
cd ..\SideWeave-Plugins
npm ci
npm run build:plugins
```

插件 UI 使用独立 Vite 入口。manifest 的 `ui.entry` 必须指向构建后的详情页，资源路径必须位于插件自己的资源根目录内。

Web 插件使用 `agile-plugin://localhost/<plugin-id>/<asset-path>` 资源协议。宿主会校验插件 ID、`assetRoot`、相对路径和规范化后的资源位置，再提供 HTML、CSS、JavaScript、图片或字体；插件 UI 必须通过 `window.agileTiles` 调用宿主能力，不能直接调用 Tauri API。

### iframe 暂停与恢复

详情页只显示当前插件 iframe。其他 iframe 会从渲染树隐藏，宿主通过 UI SDK 的 `onPluginLifecycle(handler)` 通知插件暂停和恢复。插件应在收到 `false` 时停止轮询、计时器和持续动画；收到 `true` 时刷新可能过期的数据并恢复必要活动。回调会立即收到当前状态，组件卸载时调用返回的取消函数。

```ts
import { onUnmounted } from 'vue'
import { onPluginLifecycle } from '../../../ui-sdk/runtime'

const stopLifecycleListener = onPluginLifecycle((active) => {
  if (active) startPolling()
  else stopPolling()
})
onUnmounted(stopLifecycleListener)
```

插件 UI 作为顶层页面独立运行时，SDK 会跟随页面可见性；嵌入详情页时以宿主发送的状态为准。
纯 HTML 插件可监听 `message`，只接受 `event.source === window.parent`、`__agileTiles === true`、`type === 'lifecycle'` 且 `plugin_id` 匹配的消息，并按 `active` 字段暂停或恢复。

官方插件 monorepo 的每个 `plugins/<id>/manifest.json` 是包元数据的来源。`tools/build-plugin-packages.ps1` 从 manifest 和 Cargo target 构建独立 ZIP，产物保存在 `SideWeave-Plugins/dist/packages/`。通过插件中心的本地 ZIP 导入入口安装开发包；不要直接复制 DLL 或为开发包写入 `.trusted-local`。详情窗口外壳属于宿主，详情页 HTML、Vue/React 代码、CSS 和资源属于插件。

## 插件详情页布局

插件可在 `ui.entry` 对应的 iframe 中自行实现整个详情内容。默认由宿主显示插件名称和图标；在 manifest 的 `ui` 中设置 `"chrome": "plugin"` 可隐藏这段宿主标题栏，让插件 UI 使用完整内容区域。宿主仍负责关闭、最小化和窗口拖动控件。省略 `chrome` 或设置为 `"host"` 时使用默认标题栏。Rust 插件可设置 `PluginUi.chrome` 为 `Some(PluginUiChrome::Plugin)`。

## 悬停预览

插件可以在 `hoverPreview` 中选择宿主数据预览或自定义预览页，两种方式不能同时设置：

- `dataMethod` 返回的 `items` 使用宿主的紧凑表格预览。用户可在外观设置中调整同时显示的条数和宽度；宿主按内容增高卡片，屏幕空间不足时列表滚动。`width` 作为默认宽度，用户宽度设置优先；此模式下 `height` 不再限制卡片高度。
- `entry` 指向插件包内的自定义 HTML 页面。插件负责页面内容和内部样式，清单中的 `width`、`height` 定义该页面的视口尺寸，最终窗口仍会限制在屏幕工作区内。

宿主会在预览数据和皮肤上下文的 `hostLayout` 中提供最终视口宽高、模式和有效条数上限。自定义插件页面通过 iframe 视口尺寸响应布局；自定义预览皮肤可以读取 `hostLayout` 并选择自己的排版。

```json
"hoverPreview": {
  "width": 360,
  "height": 220,
  "entry": "preview.html"
}
```

宿主外观设置中的预览显示模式默认为 `default`，也可设为 `compact`。自定义预览页的内容和样式仍由插件控制；页面可监听父窗口发来的 `preview-context` 消息，根据 `displayMode` 自行调整布局，也可以忽略该消息。消息包含 `__agileTiles: true`、匹配当前插件的 `plugin_id`，以及 `displayMode: "default" | "compact"`；页面加载和设置变化时都会收到当前值。

预览皮肤也可从上下文中的 `hostLayout.displayMode` 读取当前模式。

```js
window.addEventListener('message', (event) => {
  const message = event.data
  if (event.source !== window.parent || message?.__agileTiles !== true ||
      message.type !== 'preview-context' || message.plugin_id !== window.__agileTilesPluginId) return

  document.documentElement.dataset.previewDisplayMode = message.displayMode
})
```

## 简洁预览中的状态字段

`statusText` 是默认预览中的通用状态徽标文案，旧插件可以继续使用。需要在简洁模式显示的核心状态请额外提供 `stateText`；状态颜色仍由 `status`（如 `online`、`offline` 或 `error`）控制。默认模式优先显示 `statusText`，缺省时回退到 `stateText`；简洁模式只显示 `stateText`，并隐藏 `statusText` 和 `metrics`，不会根据文案内容判断状态或统计。

```rust
json!({
    "status": "online",
    "stateText": "已连接",
    "statusText": format!("已连接（{connections} 条链路）"),
    "metrics": metrics,
})
```

条目数等汇总信息可继续放在 `statusText` 或 `metrics` 中，因此只会在默认模式显示。旧插件无需立即迁移；要让核心状态出现在简洁模式，请将状态文案声明为 `stateText`。

声明式预览在当前模式下没有可见内容时，宿主不会弹出预览窗口。只有标题或副标题、空 `items` 数组，或简洁模式下被隐藏的统计文案和指标，都不单独算作内容。自定义 `entry` 页面仍由插件绘制，宿主不检查 iframe 内部内容。

## UI 状态与 `plugin_call` 边界

iframe 内不涉及外部副作用的操作应完全由插件前端处理，不调用 `plugin_call`：

- 打开或关闭新增/编辑表单
- 输入框、文本域、搜索框和草稿状态
- 表单校验、选项卡、筛选和排序
- 显示或隐藏插件自己的弹窗

需要保存数据或调用宿主能力时才使用 `plugin_call` / `window.agileTiles.plugin`：

- 读取、保存、删除插件数据
- 文件选择、图标提取、启动进程或打开外部 URL
- 创建或控制独立桌面窗口
- 跨窗口事件和跨插件通信

例如启动器的 `openAdd` / `openEdit` 只是前端状态，不应调用后端；列表初始化、保存应用、选择 exe 和启动应用则必须调用后端。便签“新建”如果直接创建并持久化独立桌面便签，不属于纯表单操作，因此仍需要调用后端；如果改成 iframe 内草稿表单，应在最终保存时再调用创建接口。

## 独立 DLL 插件包

可信本地 Windows 插件默认使用 `dynamic_library` runtime。宿主启动时遍历 `%APPDATA%/com.agiletiles.app/user_plugins/<id>`，校验 `.trusted-local`、manifest 和 `plugin.dll` 后，在主进程内加载 DLL。DLL 必须通过 SDK 导出的 `sideweave_plugin_entry` 入口，不要跨 DLL 传递 Rust trait 对象或 Tauri 类型。

```text
my-plugin/
├── manifest.json
├── plugin.dll
└── ui/
    ├── index.html
    ├── assets/
    └── ...
```

```json
{
  "manifestVersion": 1,
  "apiVersion": 1,
  "id": "my-plugin",
  "name": "My Plugin",
  "icon": "ui/icon.svg",
  "version": "1.0.0",
  "type": "native",
  "publisher": "your-publisher-id",
  "key_id": "main-2026",
  "runtime": "dynamic_library",
  "backend": "plugin.dll",
  "trust": "trusted_local",
  "requires_trust": true,
  "capabilities": ["storage"],
  "ui": {
    "entry": "ui/index.html",
    "kind": "vue",
    "assetRoot": "ui"
  }
}
```

构建官方插件包：

```powershell
npm --prefix ..\SideWeave-Plugins run build:plugins
```

更新时沿用同一插件 ID 并递增版本。插件中心会替换 `user_plugins/<id>` 中的包文件，私有数据保存在 `plugin-data/<id>`。

## 独立外部插件包

需要脱离宿主独立更新的 Windows 插件使用 `external_process` runtime：

```text
my-plugin/
├── manifest.json
├── backend/my-plugin.exe
└── ui/
    ├── index.html
    ├── assets/
    └── ...
```

```json
{
  "manifestVersion": 1,
  "apiVersion": 1,
  "id": "my-plugin",
  "name": "My Plugin",
  "icon": "ui/icon.svg",
  "version": "1.0.0",
  "type": "native",
  "publisher": "your-publisher-id",
  "key_id": "main-2026",
  "runtime": "external_process",
  "backend": "backend/my-plugin.exe",
  "trust": "trusted_local",
  "requires_trust": true,
  "capabilities": ["storage"],
  "ui": {
    "entry": "ui/index.html",
    "kind": "html",
    "assetRoot": "ui"
  }
}
```

宿主会校验 `backend` 和 `ui.entry` 必须是包内相对路径，外部进程以插件包目录为工作目录启动，详情页资源通过 `agile-plugin://` 加载。UI 构建工具需要使用相对资源路径（例如 Vite 的 `base: './'`），不能依赖宿主的 `/assets` 根路径。可信插件可以在自己的进程中使用 Windows API，但仍通过 SDK/RPC 入口与宿主交互。

zip 根目录必须直接包含 `manifest.json`、`backend/...` 和 `ui/...`，不要再包一层仓库目录。宿主启动 backend 时会传入 `--pipe <named-pipe>` 和 `--plugin-id <id>`；进程先用 `plugin-ipc` 读取 `PluginWireMessage::Handshake`，回复握手后处理 `PluginRequest`。`SideWeave-Plugins/plugins/ssh-process` 可作为外部进程适配器示例。

本地导入成功后，插件放在 `%APPDATA%/com.agiletiles.app/user_plugins/<id>`；重启宿主会自动发现并启动已授信的 external 包，不需要修改侧边栏主项目。

官方插件不再由安装器预置；使用插件中心的一键安装、更新或本地导入入口。市场 native 包必须通过 SHA256 和 Minisign 校验；首次安装第三方 publisher 密钥或遇到密钥变化时，应用会显示 publisher 与指纹并询问用户是否授信。Capabilities 不会限制 native 二进制直接访问系统。

## 权限声明

插件需要在 `manifest.json.capabilities` 声明自己会用到的宿主能力。宿主只允许以下清单中的权限名；清单的代码权威来源是 `sideweave-plugin-api::permissions`，包括各权限常量和 `permissions::ALL`。Rust 插件可直接引用这些常量，避免手写字符串。

| 权限名 | 用途 |
| --- | --- |
| `storage` | 读写插件自己的设置与密钥存储 |
| `notification.system` | 发送 Windows 原生通知 |
| `notification.desktop` | 显示宿主样式的桌面通知 |
| `notification.sidebar` | 显示侧边栏气泡通知 |
| `event.publish` | 向宿主事件总线发布事件 |
| `filesystem.read` | 读取宿主管理的插件私有文件 |
| `filesystem.write` | 写入或删除插件私有文件 |
| `filesystem.external_read` | 读取用户通过文件选择器选中的外部文件 |
| `clipboard` | 读取或写入系统剪贴板 |
| `process.spawn` | 通过宿主启动外部进程 |
| `network.listen` | 声明插件需要监听网络端口 |
| `network.connect` | 声明插件需要连接网络 |
| `window.create` | 通过宿主创建插件原生窗口 |
| `window.manage` | 显示、隐藏、移动或控制插件原生窗口 |
| `shortcut.global` | 注册或注销全局快捷键 |
| `system.open_external` | 使用系统默认浏览器打开链接 |
| `system.file_picker` | 打开系统文件选择器 |
| `system.icon` | 请求宿主提取文件图标 |

旧清单中的 `notification` 仍作为兼容别名接受，相当于同时声明三种通知权限；新插件应使用三个细分权限。声明只是申请能力：宿主 API 调用还需要用户在“控制中心 → 插件 → 权限”中授权。未声明的宿主能力会被拒绝，用户也不能打开插件未声明的权限。

例如 Rust 插件可以这样声明：

```rust
use sideweave_plugin_api::permissions;

let requested = vec![
    permissions::STORAGE.into(),
    permissions::NOTIFICATION_DESKTOP.into(),
];
```

`network.listen` 和 `network.connect` 当前用于声明记录；受信任的本地 DLL 或独立进程可以直接调用操作系统网络 API，宿主无法通过这些开关拦截。相同的信任边界也适用于可信本地代码直接访问文件、进程和剪贴板。Web 沙箱的本地文件和系统能力经宿主 API 执行并受权限控制。插件的 `trusted_local` 字段不能自行授予信任，最终信任来源仍由宿主决定。

原生插件清单可以用 `requiredCapabilities` 声明必须保留的 `capabilities` 子集。宿主会保持这些权限为已授权、禁用设置开关，并拒绝通过设置命令撤销；这只表达插件的必需能力，不能限制原生代码直接访问操作系统资源。

## Native backend startup policy

Native `dynamic_library` and `external_process` packages may set `autoStart` in `manifest.json`. It defaults to `true` for compatibility with older packages. Use `true` when `Activate` starts behavior that must run before the user opens the plugin UI (for example, a global shortcut or a persisted tunnel). Set it to `false` for on-demand plugins; the host loads the backend and sends `Activate` when the plugin first makes a `Call` or `GetInfo` request. An explicit `Activate` request also loads the backend.

```json
{
  "runtime": "dynamic_library",
  "backend": "plugin.dll",
  "autoStart": false
}
```

## Windows trusted external plugin

需要独立进程隔离的可信本地插件可以独立于宿主构建。当前 SSH sidecar 使用这条路径：

```powershell
npm run build:ssh-sidecar
npm run tauri dev
```

需要进程隔离时单独构建并打包为 `external_process` 插件；宿主与进程之间使用 SDK wire protocol 和 Windows Named Pipe。已废弃的 Gateway/local-proxy 不再属于官方构建或发布路径。

Cloudflare Tunnel 是新的 `dynamic_library` 官方插件，直接管理 `cloudflared` 子进程，不需要 Gateway sidecar：

```powershell
cd ..\SideWeave-Plugins
powershell -NoProfile -ExecutionPolicy Bypass -File tools/build-plugin-packages.ps1 -PluginId cloudflare_tunnel
```

将生成的 ZIP 通过插件中心本地导入并授信，然后在插件中心打开 Cloudflare Tunnel；插件可以选择 Dashboard Token 模式，或选择 credentials JSON 并维护本地 `config.yaml` 的 ingress 规则。

## 脱离 Tauri 测试

SDK 提供可选的 `mock` feature：

```toml
sideweave-plugin-api = { git = "https://github.com/suzikuo/SideWeave.git", rev = "<reviewed-sdk-revision>" }
sideweave-plugin-sdk = { git = "https://github.com/suzikuo/SideWeave.git", rev = "<reviewed-sdk-revision>", features = ["mock"] }
```

测试中用 `MockHostServices` 创建 `PluginContext`，不需要启动 Tauri。插件协议还提供 `PluginWireMessage::encode/decode`，外部进程可以直接使用大小受限的 JSON frame。

## 独立仓库

官方插件源码、UI SDK 和包构建工具位于 `suzikuo/SideWeave-Plugins`。API、Rust SDK、IPC 协议和宿主 runtime 保留在 SideWeave；Rust 插件依赖同一经审查 revision 的 `sideweave-plugin-api` 与 `sideweave-plugin-sdk`，由 `Cargo.lock` 固定。发布包包含 backend、根目录 `manifest.json` 和完整 UI 资源，并登记到 SideWeave-Marketplace。仓库职责和 tag 约定见 `docs/PLUGIN_REPOSITORY.md`。
