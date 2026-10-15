# SideWeave

**一款可扩展的 Windows 桌面侧边栏工作台。**把常用工具、插件和个性化界面放在屏幕边缘，需要时随手打开。

SideWeave 基于 Rust、Tauri 2 和 Vue 3 构建。它由桌面宿主、独立插件、可安装皮肤和静态插件市场组成，适合想把常用桌面操作集中起来，也希望自己开发插件扩展的人。

> A customizable Windows desktop sidebar for quick access to everyday tools, plugins, and personalized workspaces.

## 产品特色

- **随手可用的侧栏**：停靠在屏幕左侧或右侧，支持鼠标悬停展开、自动收起、全局快捷键和插件悬浮预览。侧栏宽度、图标大小、位置、透明度和展开行为都可以调整。
- **按需组合插件**：启用需要的插件、调整顺序或从侧栏隐藏；插件详情在独立区域打开，不必把所有工具塞进一个窗口。
- **可换的完整界面**：使用主题、强调色和皮肤调整视觉风格。皮肤可以覆盖侧栏、详情页、控制中心和悬浮预览，并可自带图片、字体、CSS 与交互。
- **权限先看再安装**：市场条目会展示插件声明的宿主能力；安装原生插件前会展示 publisher、密钥指纹和信任提示。
- **本地数据管理**：通过控制中心调整设置、通知、启动行为和快捷键；支持导出与导入应用备份。
- **适配网络环境**：应用更新显示下载进度。GitHub 市场与下载支持官方直连、内置镜像选项和自定义 HTTPS 镜像前缀。

## 随附的官方插件

官方插件通过插件中心独立安装和更新，不需要把所有插件预装进应用安装包。

| 插件 | 用途 |
| --- | --- |
| App Launcher | 管理并启动本地应用和 Chrome 应用窗口 |
| Cloudflare Tunnel | 管理 cloudflared Tunnel 生命周期与 ingress 配置 |
| SSH | 收藏和分组 SSH 会话、测试连通性并打开终端 |
| 便签 | 创建文本便签和任务清单，并使用独立便签窗口 |
| 端口转发 | 管理本地 TCP 端口转发规则并快速启停 |
| 工具箱 | 使用 JSON、Base64、时间戳、UUID 和正则等开发小工具 |
| 剪贴板 | 记录、搜索和再次复制剪贴板文本 |
| 快捷截图 | 截图、滚动长截图、标注、OCR、贴图及录屏导出 |
| 时钟 | 查看时钟，管理闹钟、倒计时和悬浮钟 |
| 书架 | 使用桌面悬浮阅读器阅读 TXT 文本并保存阅读进度 |
| 书签 | 分类收藏网页和常用链接，快速搜索并打开 |

插件清单和版本以当前市场目录为准；其他作者也可以发布自己的插件。

## 插件与皮肤生态

### 插件

插件可以通过市场安装，也可以从本地 ZIP 导入。当前支持不同运行方式：

- **Web 插件**运行在隔离的插件界面中，通过 SideWeave SDK 请求宿主能力。
- **Native 插件**可作为 Windows DLL 加载，或作为独立进程运行。独立进程适合需要进程隔离的插件。

插件在 `manifest.json` 中声明 `capabilities`。宿主 API 会依据用户授予的权限进行授权。市场安装页面会在安装前显示权限；旧版 `permissions` 字段仍可读取。

Native 插件具有系统级代码执行能力。权限清单不能阻止原生程序绕过宿主 API 直接访问文件、网络或启动进程。因此 SideWeave 会对第三方 publisher 和密钥变化请求明确授信；只安装你信任的原生插件。

外部开发者可从同一个独立模板仓库选择运行时：

- [Web 插件模板](https://github.com/suzikuo/SideWeave-plugin-template)
- [Native DLL 插件模板](https://github.com/suzikuo/SideWeave-plugin-template/tree/master/native)
- [插件 SDK 与仓库说明](docs/PLUGIN_REPOSITORY.md)
- [插件开发指南](docs/PLUGIN_DEVELOPMENT.md)

### 皮肤

完整皮肤包可以提供侧栏、详情、控制中心和悬浮预览四个界面，也可以附带自己的资源和推荐透明度设置。皮肤可通过市场安装，也可从本地 ZIP 导入。

参阅[皮肤开发指南](docs/SKIN_DEVELOPMENT.md)。

## 市场、签名与更新

SideWeave 使用独立的 [SideWeave Marketplace](https://github.com/suzikuo/SideWeave-Marketplace) 仓库维护插件、皮肤目录和 publisher 公钥。

- 客户端读取静态 JSON 目录，不调用 GitHub API；启动时可先显示最近有效的本地缓存，再后台刷新。
- 每个版本在目录中记录 ZIP、Minisign 签名和 SHA-256 文件的精确 HTTPS 地址。客户端不会从仓库名或 tag 推算下载地址。
- 安装前校验摘要、签名、publisher 密钥以及包内 ID、版本和类型；无效包会被拒绝。
- 应用更新使用独立的 Tauri updater 签名和固定的 `update-manifest` Release 清单，与插件 publisher 密钥分开管理。
- GitHub 镜像只负责转发访问；插件签名和应用 updater 签名仍由客户端验证。

目录中的 `verified` 表示维护者审核过该条目，不代表插件绝对安全，尤其不代表 Native 插件受沙箱限制。

## 安装

当前提供 Windows x64 安装包。请从 [GitHub Releases](https://github.com/suzikuo/SideWeave/releases/latest) 下载并安装 SideWeave。

SideWeave 保留原应用标识 `com.agiletiles.app`，以兼容既有设置和本地数据。使用旧 OSS 更新链的用户需要先手动安装一次支持 GitHub updater 的新版，之后可使用应用内更新。

## 本地开发

### 环境

- Windows x64
- Node.js 22
- Rust stable 与 MSVC C++ 构建工具
- WebView2 Runtime

### 运行宿主

```powershell
git clone https://github.com/suzikuo/SideWeave.git
cd SideWeave
npm ci
npm run tauri dev
```

构建前端或桌面安装包：

```powershell
npm run build
npm run tauri build
```

本地开发官方插件可运行 `npm run build:plugins` 生成 ZIP，再从应用插件中心导入并授信。官方插件源码位于相邻的 SideWeave-Plugins 仓库。

### 本地打包与发布

交互式发布向导支持本地打包应用、插件和皮肤，也支持上传到 GitHub Releases：

```powershell
.\scripts\publish-wizard.ps1
```

插件和皮肤上传成功后，向导可以继续更新 Marketplace 的 `marketplace.json`，校验目录并提交推送。签名私钥只保存在维护者本机，不应提交到仓库。

完整流程请参阅[发布指南](docs/PUBLISHING.md)和[市场协议](docs/MARKET.md)。

## 项目结构

```text
src/                 Vue 前端与控制中心
src-tauri/           Tauri 宿主、桌面窗口和系统集成
crates/              插件 API、SDK、IPC 与运行时 crate
../SideWeave-Plugins/  官方插件源代码与构建工具（独立仓库）
  plugins/             官方插件源代码
  skins/               官方皮肤源代码与打包工具
docs/                 开发、市场和发布文档
scripts/              本地构建、打包和发布工具
```

官方插件、皮肤与外部包都按可独立安装的签名包发布。Rust 插件依赖 `sideweave-plugin-api` 定义协议类型，依赖 `sideweave-plugin-sdk` 使用宿主能力；独立进程插件还需 `sideweave-plugin-ipc`。Web、Native 和 Skin 第三方模板都位于 `suzikuo/SideWeave-plugin-template`，分别放在仓库根目录、`native/` 和 `skin/`。插件不得依赖 `src-tauri` 宿主内部实现。

## 文档

- [插件开发](docs/PLUGIN_DEVELOPMENT.md)
- [插件仓库与 SDK 依赖](docs/PLUGIN_REPOSITORY.md)
- [皮肤开发](docs/SKIN_DEVELOPMENT.md)
- [Marketplace 协议](docs/MARKET.md)
- [Windows 本地构建与发布](docs/PUBLISHING.md)
- [开发概览](docs/DEVELOP.md)

## 技术栈

- [Rust](https://www.rust-lang.org/) 与 [Tauri 2](https://tauri.app/)：桌面宿主和系统集成
- [Vue 3](https://vuejs.org/) 与 [Vite](https://vite.dev/)：应用界面与插件 UI
- [GitHub Releases](https://docs.github.com/repositories/releasing-projects-on-github)：应用、插件和皮肤的静态分发
- [Minisign](https://jedisct1.github.io/minisign/) 与 SHA-256：插件包签名和完整性校验

## 许可协议

SideWeave 使用 [MIT License](LICENSE)。
