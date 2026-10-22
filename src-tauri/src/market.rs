//! 插件中心与皮肤中心后端：
//! - 市场：读取 GitHub 上的静态目录与发布者密钥目录
//! - 安装：从目录中的精确 URL 下载 ZIP、摘要和签名，验证后解压到用户目录
//!   - 插件：user_plugins/<id>/（manifest.json + backend + 插件自有 UI 资源）
//!   - 皮肤：user_skins/<id>/（manifest.json + 可选 skin.css / skinUi 资源）
//! - 移除：删除对应目录
//! Web 插件以沙箱 iframe 渲染（无 Tauri 权限），native 插件则通过受信任的 DLL/独立进程运行；两者都使用统一 Host API 桥接。

use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use url::Url;

use sideweave_plugin_api::{
    ErrorCode, PluginError, PluginId, PluginManifest, PluginRequest, PluginRequestKind,
    PluginResponse, PluginRuntime, PluginTrust, API_VERSION, MANIFEST_VERSION,
};
use serde::{Deserialize, Serialize};
use sha2::Digest;
use tauri::http::{header, Request, Response, StatusCode};
use tauri::{AppHandle, Manager, Runtime};

use crate::paths;
use tauri_plugin_dialog::DialogExt as _;

// ---------- 市场索引 ----------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct MarketItem {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    pub publisher: String,
    #[serde(rename = "type")]
    pub package_type: String,
    #[serde(default)]
    pub repository: String,
    #[serde(default)]
    pub verified: bool,
    pub latest: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    pub versions: Vec<MarketVersion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketVersion {
    pub version: String,
    pub key_id: String,
    pub release: MarketRelease,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketRelease {
    pub tag: String,
    pub asset: String,
    pub url: String,
    pub signature_url: String,
    pub checksum_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct MarketIndex {
    pub schema_version: u32,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub plugins: Vec<MarketItem>,
    #[serde(default)]
    pub skins: Vec<MarketItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PublisherDirectory {
    pub schema_version: u32,
    pub publishers: Vec<Publisher>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Publisher {
    pub id: String,
    pub name: String,
    pub keys: Vec<PublisherKey>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PublisherKey {
    pub id: String,
    pub algorithm: String,
    pub public_key: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
struct MarketCache {
    schema_version: u32,
    marketplace: MarketIndex,
    publishers: PublisherDirectory,
    marketplace_etag: Option<String>,
    publishers_etag: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketInstallInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub publisher: String,
    pub publisher_name: String,
    pub package_type: String,
    pub verified: bool,
    pub native: bool,
    pub trust_required: bool,
    pub key_id: String,
    pub key_fingerprint: String,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketTrustApproval {
    pub publisher: String,
    pub key_id: String,
    pub key_fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
struct TrustedPublisherKey {
    publisher: String,
    key_id: String,
    fingerprint: String,
}

const MARKETPLACE_SCHEMA_VERSION: u32 = 1;
const MARKETPLACE_URL: &str = "https://raw.githubusercontent.com/suzikuo/SideWeave-Marketplace/master/marketplace.json";
const PUBLISHERS_URL: &str = "https://raw.githubusercontent.com/suzikuo/SideWeave-Marketplace/master/publishers.json";
const OFFICIAL_PUBLISHER_ID: &str = "sideweave";
// This key is compiled into the application through the Actions variable and is never read from the marketplace.
const OFFICIAL_PUBLISHER_PUBLIC_KEY: Option<&str> = option_env!("SIDEWEAVE_OFFICIAL_PUBLISHER_KEY");

/// Apply the selected GitHub proxy prefix only to GitHub-hosted URLs. The catalog continues to
/// provide the canonical, exact asset URL; this only changes its transport endpoint.
pub(crate) fn mirrored_github_url(mirror_prefix: &str, source_url: &str) -> Result<String, String> {
    let mirror_prefix = crate::settings::validate_github_mirror(mirror_prefix)?;
    let source = Url::parse(source_url).map_err(|error| format!("GitHub URL 无效: {error}"))?;
    let Some(host) = source.host_str() else {
        return Err("GitHub URL 缺少主机名".into());
    };
    let is_github = host.eq_ignore_ascii_case("github.com")
        || host.eq_ignore_ascii_case("githubusercontent.com")
        || host.to_ascii_lowercase().ends_with(".githubusercontent.com");
    if mirror_prefix.is_empty() || !is_github {
        return Ok(source_url.to_string());
    }
    let source_without_scheme = source_url
        .split_once("://")
        .map(|(_, remainder)| remainder)
        .ok_or_else(|| "GitHub URL 缺少协议头".to_string())?;
    Ok(format!("{mirror_prefix}{source_without_scheme}"))
}

// ---------- 已安装清单 ----------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Manifest {
    #[serde(default = "default_manifest_version")]
    pub manifest_version: u32,
    #[serde(default = "default_api_version")]
    pub api_version: u32,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub icon: String,
    pub version: String,
    #[serde(default, rename = "type")]
    pub package_type: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    /// 插件入口 html（单文件）；皮肤包无此字段
    #[serde(default)]
    pub entry: String,
    /// Plugin-owned detail page metadata. `entry` remains as a legacy fallback.
    #[serde(default)]
    pub ui: Option<sideweave_plugin_api::PluginUi>,
    /// Optional full-window UI replacements for a skin package.
    #[serde(default, rename = "skinUi")]
    pub skin_ui: Option<SkinUi>,
    /// Recommended host opacity values used by the appearance reset action.
    #[serde(default, rename = "appearanceDefaults")]
    pub appearance_defaults: Option<AppearanceOpacityDefaults>,
    /// Runtime/trust/permissions use the shared plugin manifest contract.
    #[serde(default)]
    pub runtime: Option<PluginRuntime>,
    /// External-process backend relative to the installed plugin directory.
    #[serde(default)]
    pub backend: Option<String>,
    #[serde(default)]
    pub trust: Option<PluginTrust>,
    /// Start the backend after the host UI is ready; legacy packages stay enabled by default.
    #[serde(default = "default_auto_start", alias = "autoStart")]
    pub auto_start: bool,
    #[serde(default, rename = "capabilities", alias = "permissions")]
    pub permissions: Vec<String>,
    #[serde(default, rename = "requiredCapabilities")]
    pub required_permissions: Vec<String>,
    #[serde(default)]
    pub publisher: String,
    #[serde(default, rename = "key_id")]
    pub key_id: String,
    #[serde(default)]
    pub requires_trust: bool,
    #[serde(default, alias = "hoverPreview")]
    pub hover_preview: Option<sideweave_plugin_api::PluginHoverPreview>,
    #[serde(default)]
    pub minimum_host_version: Option<String>,
}

/// UI entry points for a skin. Each entry is relative to `asset_root`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct SkinUi {
    pub asset_root: String,
    pub sidebar: String,
    pub detail: String,
    pub control: String,
    pub preview: String,
}

/// Optional skin-specific opacity recommendations. Unspecified values use host defaults.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct AppearanceOpacityDefaults {
    pub sidebar_panel_opacity: Option<f64>,
    pub detail_panel_opacity: Option<f64>,
    pub control_panel_opacity: Option<f64>,
    pub preview_panel_opacity: Option<f64>,
    pub preview_image_opacity: Option<f64>,
    pub sidebar_image_opacity: Option<f64>,
    pub detail_image_opacity: Option<f64>,
    pub component_opacity: Option<f64>,
}

impl AppearanceOpacityDefaults {
    fn clamp_values(&mut self) {
        for value in [
            &mut self.sidebar_panel_opacity,
            &mut self.detail_panel_opacity,
            &mut self.control_panel_opacity,
            &mut self.preview_panel_opacity,
            &mut self.preview_image_opacity,
            &mut self.sidebar_image_opacity,
            &mut self.detail_image_opacity,
            &mut self.component_opacity,
        ] {
            if let Some(value) = value {
                *value = value.clamp(0.0, 1.0);
            }
        }
    }

    fn validate(&self) -> Result<(), String> {
        for value in [
            self.sidebar_panel_opacity,
            self.detail_panel_opacity,
            self.control_panel_opacity,
            self.preview_panel_opacity,
            self.preview_image_opacity,
            self.sidebar_image_opacity,
            self.detail_image_opacity,
            self.component_opacity,
        ]
        .into_iter()
        .flatten()
        {
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                return Err("appearanceDefaults 中的不透明度必须在 0 到 1 之间".into());
            }
        }
        Ok(())
    }
}

fn default_manifest_version() -> u32 {
    MANIFEST_VERSION
}

fn default_api_version() -> u32 {
    API_VERSION
}

fn default_auto_start() -> bool {
    true
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledItem {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    /// "plugin" | "skin"
    pub kind: String,
    /// Installed runtime. Web plugins open inside the sandbox route; native runtimes use the detail window.
    pub runtime: Option<PluginRuntime>,
    /// 有更新时的云端版本
    pub update_version: Option<String>,
    /// Per-skin recommended values for the opacity reset action.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub appearance_defaults: Option<AppearanceOpacityDefaults>,
}

fn user_plugins_dir(app_data: &Path) -> PathBuf {
    app_data.join("user_plugins")
}
fn user_skins_dir(app_data: &Path) -> PathBuf {
    app_data.join("user_skins")
}

fn is_staging_dir(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with(".import-"))
}

/// 清理安装过程中断电、崩溃或进程被终止后遗留的临时目录。
/// 临时目录不能进入市场列表或运行时扫描，否则会制造同一插件的重复条目。
pub fn cleanup_staging_dirs(app_data: &Path) {
    for base in [user_plugins_dir(app_data), user_skins_dir(app_data)] {
        let Ok(entries) = std::fs::read_dir(base) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if is_staging_dir(&path) && path.is_dir() {
                let _ = std::fs::remove_dir_all(path);
            }
        }
    }
}

fn read_manifest(dir: &Path) -> Option<Manifest> {
    paths::load_json(&dir.join("manifest.json")).and_then(|v| serde_json::from_value(v).ok())
}

fn plugin_ui_entry(manifest: &Manifest) -> Option<&str> {
    manifest
        .ui
        .as_ref()
        .map(|ui| ui.entry.as_str())
        .filter(|entry| !entry.is_empty())
        .or_else(|| (!manifest.entry.is_empty()).then_some(manifest.entry.as_str()))
}

const TRUSTED_LOCAL_MARKER: &str = ".trusted-local";
const RETIRED_PLUGIN_IDS: &[&str] = &["local_proxy", "gateway"];

pub fn is_trusted_local_plugin(dir: &Path) -> bool {
    dir.join(TRUSTED_LOCAL_MARKER).is_file()
}

pub fn plugin_backend_path(dir: &Path, manifest: &PluginManifest) -> Option<PathBuf> {
    if !matches!(
        manifest.runtime,
        PluginRuntime::ExternalProcess | PluginRuntime::DynamicLibrary
    ) {
        return None;
    }
    let backend = manifest.backend.as_deref()?;
    if !safe_relative_path(backend) {
        return None;
    }
    let package_root = std::fs::canonicalize(dir).ok()?;
    let backend_path = std::fs::canonicalize(dir.join(backend)).ok()?;
    backend_path
        .starts_with(&package_root)
        .then_some(backend_path)
}

fn safe_relative_path(value: &str) -> bool {
    !value.is_empty()
        && Path::new(value).is_relative()
        && Path::new(value)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

fn compare_versions(left: &str, right: &str) -> Ordering {
    let (left_core, left_pre) = left.split_once('-').unwrap_or((left, ""));
    let (right_core, right_pre) = right.split_once('-').unwrap_or((right, ""));
    let left_parts: Vec<u64> = left_core
        .split('.')
        .map(|part| part.parse::<u64>().unwrap_or(0))
        .collect();
    let right_parts: Vec<u64> = right_core
        .split('.')
        .map(|part| part.parse::<u64>().unwrap_or(0))
        .collect();
    for index in 0..left_parts.len().max(right_parts.len()) {
        match left_parts
            .get(index)
            .copied()
            .unwrap_or(0)
            .cmp(&right_parts.get(index).copied().unwrap_or(0))
        {
            Ordering::Equal => {}
            order => return order,
        }
    }
    match (left_pre.is_empty(), right_pre.is_empty()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => {
            let left_parts: Vec<&str> = left_pre.split('.').collect();
            let right_parts: Vec<&str> = right_pre.split('.').collect();
            for index in 0..left_parts.len().max(right_parts.len()) {
                let Some(left_part) = left_parts.get(index) else {
                    return Ordering::Less;
                };
                let Some(right_part) = right_parts.get(index) else {
                    return Ordering::Greater;
                };
                let order = match (left_part.parse::<u64>(), right_part.parse::<u64>()) {
                    (Ok(left), Ok(right)) => left.cmp(&right),
                    (Ok(_), Err(_)) => Ordering::Less,
                    (Err(_), Ok(_)) => Ordering::Greater,
                    (Err(_), Err(_)) => left_part.cmp(right_part),
                };
                if order != Ordering::Equal {
                    return order;
                }
            }
            Ordering::Equal
        }
    }
}

fn market_item<'a>(items: &'a [MarketItem], id: &str) -> Option<&'a MarketItem> {
    items.iter().find(|item| item.id == id)
}

fn latest_market_version(item: &MarketItem) -> Option<&MarketVersion> {
    item.versions.iter().find(|version| version.version == item.latest)
}

fn is_https_url(value: &str) -> bool {
    url::Url::parse(value).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
    })
}

fn validate_item(item: &MarketItem, kind: &str) -> Result<(), String> {
    if !valid_id(&item.id) || item.name.trim().is_empty() || !valid_id(&item.publisher) {
        return Err(format!("市场条目 {} 的 id、name 或 publisher 无效", item.id));
    }
    let expected_type = if kind == "skin" {
        item.package_type == "skin"
    } else {
        matches!(item.package_type.as_str(), "web" | "native")
    };
    if !expected_type {
        return Err(format!("市场条目 {} 的 type 不受支持", item.id));
    }
    if item.versions.is_empty() {
        return Err(format!("市场条目 {} 没有任何版本", item.id));
    }
    let mut versions = std::collections::HashSet::new();
    for version in &item.versions {
        if version.version.trim().is_empty() || !versions.insert(version.version.as_str()) {
            return Err(format!("市场条目 {} 包含空或重复版本", item.id));
        }
        let release = &version.release;
        if release.tag.trim().is_empty()
            || release.asset.trim().is_empty()
            || !is_https_url(&release.url)
            || !is_https_url(&release.signature_url)
            || !is_https_url(&release.checksum_url)
        {
            return Err(format!("市场条目 {}@{} 的 Release 资产信息无效", item.id, version.version));
        }
    }
    if !versions.contains(item.latest.as_str()) {
        return Err(format!("市场条目 {} 的 latest 未指向 versions 中的版本", item.id));
    }
    Ok(())
}

fn validate_directories(index: &MarketIndex, publishers: &PublisherDirectory) -> Result<(), String> {
    if index.schema_version != MARKETPLACE_SCHEMA_VERSION
        || publishers.schema_version != MARKETPLACE_SCHEMA_VERSION
    {
        return Err("市场目录 schema_version 不受支持".into());
    }
    let mut ids = std::collections::HashSet::new();
    for (kind, items) in [("plugin", &index.plugins), ("skin", &index.skins)] {
        for item in items {
            if !ids.insert((kind, item.id.as_str())) {
                return Err(format!("市场目录中存在重复条目 {}", item.id));
            }
            validate_item(item, kind)?;
        }
    }
    let mut publisher_ids = std::collections::HashSet::new();
    for publisher in &publishers.publishers {
        if !valid_id(&publisher.id) || publisher.name.trim().is_empty() || !publisher_ids.insert(&publisher.id) {
            return Err(format!("发布者 {} 无效或重复", publisher.id));
        }
        let mut key_ids = std::collections::HashSet::new();
        for key in &publisher.keys {
            if key.id.trim().is_empty()
                || !key_ids.insert(&key.id)
                || key.algorithm != "minisign"
                || key.public_key.trim().is_empty()
                || !matches!(key.status.as_str(), "active" | "retired" | "revoked")
            {
                return Err(format!("发布者 {} 的密钥目录无效", publisher.id));
            }
            if decode_minisign_public_key(&key.public_key).is_err() {
                return Err(format!("发布者 {}/{} 的 Minisign 公钥无效", publisher.id, key.id));
            }
        }
    }
    for item in index.plugins.iter().chain(index.skins.iter()) {
        let Some(publisher) = publishers.publishers.iter().find(|p| p.id == item.publisher) else {
            return Err(format!("市场条目 {} 引用了未知发布者 {}", item.id, item.publisher));
        };
        if publisher.keys.is_empty() {
            return Err(format!("发布者 {} 没有可验证的密钥", item.publisher));
        }
        for version in &item.versions {
            if !publisher.keys.iter().any(|key| key.id == version.key_id) {
                return Err(format!("市场条目 {}@{} 引用了未知 key_id", item.id, version.version));
            }
        }
    }
    Ok(())
}

pub fn normalize_market_index(mut index: MarketIndex) -> MarketIndex {
    index.plugins.sort_by(|left, right| left.name.cmp(&right.name));
    index.skins.sort_by(|left, right| left.name.cmp(&right.name));
    index
}

fn shared_manifest(manifest: &Manifest) -> Result<PluginManifest, String> {
    let id = PluginId::new(manifest.id.clone()).map_err(|error| error.to_string())?;
    let runtime = manifest.runtime.unwrap_or(PluginRuntime::WebSandbox);
    let ui = plugin_ui_entry(manifest).and_then(|entry| {
        safe_relative_path(entry).then(|| {
            let mut ui = manifest.ui.clone().unwrap_or_default();
            ui.entry = entry.to_string();
            if ui.asset_root.is_empty() {
                ui.asset_root = ".".into();
            }
            ui
        })
    });
    let shared = PluginManifest {
        manifest_version: manifest.manifest_version,
        api_version: manifest.api_version,
        id,
        name: manifest.name.clone(),
        icon: manifest.icon.clone(),
        version: manifest.version.clone(),
        description: manifest.description.clone(),
        author: manifest.author.clone(),
        runtime,
        backend: manifest.backend.clone(),
        trust: manifest.trust.unwrap_or_default(),
        auto_start: manifest.auto_start,
        permissions: manifest.permissions.clone(),
        required_permissions: manifest.required_permissions.clone(),
        ui,
        hover_preview: manifest.hover_preview.clone(),
        minimum_host_version: manifest.minimum_host_version.clone(),
    };
    shared.validate().map_err(|error| error.to_string())?;
    Ok(shared)
}

// ---------- GitHub 静态目录与本地缓存 ----------

fn cache_path<R: Runtime>(app: &tauri::AppHandle<R>) -> PathBuf {
    paths::app_data_dir(app).join("marketplace-cache.json")
}

fn load_market_cache<R: Runtime>(app: &tauri::AppHandle<R>) -> Option<MarketCache> {
    let cache: MarketCache = paths::load_json(&cache_path(app))
        .and_then(|value| serde_json::from_value(value).ok())?;
    if cache.schema_version != MARKETPLACE_SCHEMA_VERSION
        || validate_directories(&cache.marketplace, &cache.publishers).is_err()
    {
        return None;
    }
    Some(cache)
}

pub fn market_snapshot<R: Runtime>(app: &tauri::AppHandle<R>) -> Result<MarketIndex, String> {
    load_market_cache(app)
        .map(|cache| normalize_market_index(cache.marketplace))
        .ok_or_else(|| "尚无有效的本地市场缓存".into())
}

async fn fetch_document(
    client: &reqwest::Client,
    url: &str,
    mirror_prefix: &str,
    etag: Option<&str>,
) -> Result<(Option<Vec<u8>>, Option<String>), String> {
    let request_url = mirrored_github_url(mirror_prefix, url)?;
    let mut request = client.get(request_url);
    if let Some(etag) = etag {
        request = request.header(reqwest::header::IF_NONE_MATCH, etag);
    }
    let response = request.send().await.map_err(|error| format!("无法读取市场目录: {error}"))?;
    if response.status() == reqwest::StatusCode::NOT_MODIFIED {
        return Ok((None, etag.map(str::to_string)));
    }
    let response = response
        .error_for_status()
        .map_err(|error| format!("读取市场目录失败: {error}"))?;
    let etag = response
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let bytes = response.bytes().await.map_err(|error| error.to_string())?.to_vec();
    Ok((Some(bytes), etag))
}

pub async fn refresh_market_cache<R: Runtime>(app: &tauri::AppHandle<R>) -> Result<MarketIndex, String> {
    let previous = load_market_cache(app);
    let mirror_prefix = crate::host_settings(app).general.github_mirror;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .user_agent(concat!("SideWeave/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| error.to_string())?;
    let (marketplace_bytes, marketplace_etag) = fetch_document(
        &client,
        MARKETPLACE_URL,
        &mirror_prefix,
        previous.as_ref().and_then(|cache| cache.marketplace_etag.as_deref()),
    )
    .await?;
    let (publishers_bytes, publishers_etag) = fetch_document(
        &client,
        PUBLISHERS_URL,
        &mirror_prefix,
        previous.as_ref().and_then(|cache| cache.publishers_etag.as_deref()),
    )
    .await?;

    let marketplace = match marketplace_bytes {
        Some(bytes) => serde_json::from_slice::<MarketIndex>(&bytes)
            .map_err(|error| format!("marketplace.json 格式无效: {error}"))?,
        None => previous
            .as_ref()
            .map(|cache| cache.marketplace.clone())
            .ok_or_else(|| "市场返回 304，但本机没有可用缓存".to_string())?,
    };
    let publishers = match publishers_bytes {
        Some(bytes) => serde_json::from_slice::<PublisherDirectory>(&bytes)
            .map_err(|error| format!("publishers.json 格式无效: {error}"))?,
        None => previous
            .as_ref()
            .map(|cache| cache.publishers.clone())
            .ok_or_else(|| "发布者目录返回 304，但本机没有可用缓存".to_string())?,
    };
    validate_directories(&marketplace, &publishers)?;
    let mut marketplace = normalize_market_index(marketplace);
    if marketplace.updated_at.trim().is_empty() {
        marketplace.updated_at = chrono::Utc::now().to_rfc3339();
    }
    let cache = MarketCache {
        schema_version: MARKETPLACE_SCHEMA_VERSION,
        marketplace: marketplace.clone(),
        publishers,
        marketplace_etag,
        publishers_etag,
    };
    let value = serde_json::to_value(cache).map_err(|error| error.to_string())?;
    paths::save_json_atomic(&cache_path(app), &value).map_err(|error| error.to_string())?;
    Ok(marketplace)
}

// ---------- 已安装扫描 ----------

fn scan_installed(dir: &Path, kind: &str, index: Option<&MarketIndex>) -> Vec<InstalledItem> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() || is_staging_dir(&path) {
            continue;
        }
        let Some(manifest) = read_manifest(&path) else {
            continue;
        };
        if RETIRED_PLUGIN_IDS.contains(&manifest.id.as_str()) {
            continue;
        }
        let shared = shared_manifest(&manifest).ok();
        let market_version = index.and_then(|idx| {
            let items = if kind == "plugin" {
                &idx.plugins
            } else {
                &idx.skins
            };
            market_item(items, &manifest.id).and_then(|item| latest_market_version(item)).map(|v| v.version.clone())
        });
        let runtime = shared.as_ref().map(|manifest| manifest.runtime);
        let update_version = match market_version {
            // `latest` is an explicit publisher-controlled pointer and may move backwards for rollback.
            Some(v) if v != manifest.version => Some(v),
            _ => None,
        };
        let appearance_defaults = if kind == "skin" {
            manifest.appearance_defaults.clone().map(|mut defaults| {
                defaults.clamp_values();
                defaults
            })
        } else {
            None
        };
        out.push(InstalledItem {
            id: manifest.id,
            name: manifest.name,
            version: manifest.version,
            description: manifest.description,
            author: manifest.author,
            kind: kind.into(),
            runtime,
            update_version,
            appearance_defaults,
        });
    }
    let mut deduped = HashMap::<String, InstalledItem>::new();
    for item in out {
        let replace = deduped
            .get(&item.id)
            .map(|current| compare_versions(&item.version, &current.version) == Ordering::Greater)
            .unwrap_or(true);
        if replace {
            deduped.insert(item.id.clone(), item);
        }
    }
    let mut out: Vec<InstalledItem> = deduped.into_values().collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledLists {
    pub plugins: Vec<InstalledItem>,
    pub skins: Vec<InstalledItem>,
}

pub async fn installed_lists<R: Runtime>(app: &tauri::AppHandle<R>) -> InstalledLists {
    let data = paths::app_data_dir(app);
    cleanup_staging_dirs(&data);
    let index = load_market_cache(app).map(|cache| cache.marketplace);
    InstalledLists {
        plugins: scan_installed(&user_plugins_dir(&data), "plugin", index.as_ref()),
        skins: scan_installed(&user_skins_dir(&data), "skin", index.as_ref()),
    }
}

// ---------- 安装 ----------

fn package_key<'a>(
    item: &MarketItem,
    version: &MarketVersion,
    publishers: &'a PublisherDirectory,
) -> Result<(PublisherKey, String), String> {
    let publisher = publishers
        .publishers
        .iter()
        .find(|publisher| publisher.id == item.publisher)
        .ok_or_else(|| format!("未知发布者 {}", item.publisher))?;
    let mut key = publisher
        .keys
        .iter()
        .find(|key| key.id == version.key_id)
        .cloned()
        .ok_or_else(|| format!("未知发布者密钥 {}", version.key_id))?;
    if key.status == "revoked" {
        return Err(format!("发布者密钥 {} 已撤销，拒绝安装", key.id));
    }
    if version.version == item.latest && key.status != "active" {
        return Err(format!("发布者密钥 {} 已退役，不能用于发布新版本", key.id));
    }
    if item.publisher == OFFICIAL_PUBLISHER_ID {
        let pinned = OFFICIAL_PUBLISHER_PUBLIC_KEY
            .ok_or_else(|| "应用尚未内置 SideWeave 官方插件公钥".to_string())?;
        if minisign_key_payload(&key.public_key) != minisign_key_payload(pinned) {
            return Err("市场目录中的官方公钥与应用内置公钥不一致".into());
        }
        key.public_key = pinned.to_string();
    }
    let canonical_key = minisign_key_payload(&key.public_key);
    let digest = sha2::Sha256::digest(canonical_key.as_bytes());
    let fingerprint: String = digest.iter().take(12).map(|byte| format!("{byte:02X}")).collect();
    Ok((key, fingerprint))
}

fn minisign_key_payload(public_key: &str) -> String {
    public_key
        .lines()
        .filter(|line| !line.trim_start().starts_with("untrusted comment:"))
        .map(str::trim)
        .collect::<String>()
}

fn trust_store_path<R: Runtime>(app: &tauri::AppHandle<R>) -> PathBuf {
    paths::app_data_dir(app).join("trusted-publisher-keys.json")
}

fn trusted_key<R: Runtime>(app: &tauri::AppHandle<R>, publisher: &str, key_id: &str, fingerprint: &str) -> bool {
    paths::load_json(&trust_store_path(app))
        .and_then(|value| serde_json::from_value::<Vec<TrustedPublisherKey>>(value).ok())
        .is_some_and(|keys| keys.iter().any(|key| {
            key.publisher == publisher && key.key_id == key_id && key.fingerprint == fingerprint
        }))
}

fn remember_trusted_key<R: Runtime>(
    app: &tauri::AppHandle<R>,
    publisher: &str,
    key_id: &str,
    fingerprint: &str,
) -> Result<(), String> {
    let path = trust_store_path(app);
    let mut keys = paths::load_json(&path)
        .and_then(|value| serde_json::from_value::<Vec<TrustedPublisherKey>>(value).ok())
        .unwrap_or_default();
    keys.retain(|key| !(key.publisher == publisher && key.key_id == key_id));
    keys.push(TrustedPublisherKey {
        publisher: publisher.to_string(),
        key_id: key_id.to_string(),
        fingerprint: fingerprint.to_string(),
    });
    let value = serde_json::to_value(keys).map_err(|error| error.to_string())?;
    paths::save_json_atomic(&path, &value).map_err(|error| error.to_string())
}

fn resolve_install_info<R: Runtime>(
    app: &tauri::AppHandle<R>,
    index: &MarketIndex,
    publishers: &PublisherDirectory,
    kind: &str,
    id: &str,
) -> Result<MarketInstallInfo, String> {
    let items = match kind {
        "plugin" => &index.plugins,
        "skin" => &index.skins,
        _ => return Err("未知的安装类型".into()),
    };
    let item = market_item(items, id).ok_or_else(|| format!("市场中不存在 {id}"))?;
    let version = latest_market_version(item).ok_or_else(|| format!("市场条目 {id} 没有 latest 版本"))?;
    let (key, fingerprint) = package_key(item, version, publishers)?;
    let publisher_name = publishers.publishers.iter()
        .find(|publisher| publisher.id == item.publisher)
        .map(|publisher| publisher.name.clone())
        .unwrap_or_else(|| item.publisher.clone());
    let native = kind == "plugin" && item.package_type == "native";
    let trust_required = native
        && !trusted_key(app, &item.publisher, &key.id, &fingerprint);
    Ok(MarketInstallInfo {
        id: item.id.clone(),
        name: item.name.clone(),
        version: version.version.clone(),
        publisher: item.publisher.clone(),
        publisher_name,
        package_type: item.package_type.clone(),
        verified: item.verified,
        native,
        trust_required,
        key_id: key.id,
        key_fingerprint: fingerprint,
        capabilities: item.capabilities.clone(),
    })
}

pub fn install_info<R: Runtime>(
    app: &tauri::AppHandle<R>,
    kind: &str,
    id: &str,
) -> Result<MarketInstallInfo, String> {
    let cache = load_market_cache(app)
        .ok_or_else(|| "本机没有有效的市场发布者目录，请先刷新市场".to_string())?;
    resolve_install_info(app, &cache.marketplace, &cache.publishers, kind, id)
}

async fn get_asset<R: Runtime>(
    app: &tauri::AppHandle<R>,
    client: &reqwest::Client,
    url: &str,
) -> Result<Vec<u8>, String> {
    if !is_https_url(url) {
        return Err("市场资产 URL 必须使用 HTTPS".into());
    }
    let mirror_prefix = crate::host_settings(app).general.github_mirror;
    let download_url = mirrored_github_url(&mirror_prefix, url)?;
    if !is_https_url(&download_url) {
        return Err("镜像 URL 必须使用 HTTPS".into());
    }
    client.get(download_url).send().await
        .map_err(|error| format!("下载市场资产失败: {error}"))?
        .error_for_status()
        .map_err(|error| format!("下载市场资产失败: {error}"))?
        .bytes().await.map(|bytes| bytes.to_vec()).map_err(|error| error.to_string())
}

fn parse_checksum(bytes: &[u8]) -> Result<String, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "SHA256 文件不是 UTF-8".to_string())?;
    let hash = text.split_whitespace().next().unwrap_or_default();
    if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("SHA256 文件格式无效".into());
    }
    Ok(hash.to_ascii_lowercase())
}

fn decode_minisign_public_key(public_key: &str) -> Result<minisign_verify::PublicKey, String> {
    let key = if public_key.lines().count() > 1 {
        minisign_verify::PublicKey::decode(public_key)
    } else {
        minisign_verify::PublicKey::from_base64(public_key.trim())
    }
    .map_err(|error| format!("发布者公钥无效: {error}"))?;
    Ok(key)
}

fn verify_minisign(bytes: &[u8], signature: &[u8], public_key: &str) -> Result<(), String> {
    let signature = std::str::from_utf8(signature).map_err(|_| "Minisign 签名文件格式无效".to_string())?;
    let signature = minisign_verify::Signature::decode(signature)
        .map_err(|error| format!("Minisign 签名无法解析: {error}"))?;
    let public_key = decode_minisign_public_key(public_key)?;
    public_key.verify(bytes, &signature, false)
        .map_err(|error| format!("插件包签名校验失败: {error}"))
}

fn read_package_manifest(bytes: &[u8]) -> Result<Manifest, String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|error| format!("安装包不是有效的 zip: {error}"))?;
    let mut file = archive.by_name("manifest.json")
        .map_err(|_| "安装包缺少根目录 manifest.json".to_string())?;
    let mut text = String::new();
    std::io::Read::read_to_string(&mut file, &mut text).map_err(|error| error.to_string())?;
    serde_json::from_str(&text).map_err(|error| format!("manifest.json 格式无效: {error}"))
}

fn unzip_into(zip_bytes: &[u8], dest: &Path) -> Result<(), String> {
    let reader = std::io::Cursor::new(zip_bytes);
    let mut archive =
        zip::ZipArchive::new(reader).map_err(|e| format!("安装包不是有效的 zip: {e}"))?;
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| format!("安装包读取失败: {e}"))?;
        let Some(name) = file.enclosed_name() else {
            continue; // 跳过非法路径（防 zip slip）
        };
        let out_path = dest.join(name);
        if file.is_dir() {
            std::fs::create_dir_all(&out_path).map_err(|e| e.to_string())?;
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let mut out = std::fs::File::create(&out_path).map_err(|e| e.to_string())?;
            std::io::copy(&mut file, &mut out).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

pub async fn install<R: Runtime>(
    app: &tauri::AppHandle<R>,
    kind: &str,
    id: &str,
    trust_approval: Option<MarketTrustApproval>,
    before_replace: impl FnOnce(&Manifest) -> Result<(), String>,
) -> Result<(), String> {
    if !valid_id(id) {
        return Err("非法的 id".into());
    }
    let cache = load_market_cache(app)
        .ok_or_else(|| "本机没有有效的市场发布者目录，请先刷新市场".to_string())?;
    let index = cache.marketplace;
    let publishers = cache.publishers;
    let info = resolve_install_info(app, &index, &publishers, kind, id)?;
    if info.trust_required
        && !trust_approval.as_ref().is_some_and(|approval| {
            approval.publisher == info.publisher
                && approval.key_id == info.key_id
                && approval.key_fingerprint == info.key_fingerprint
        })
    {
        return Err("安装 native 插件前必须明确授信该发布者及当前密钥".into());
    }
    let items = if kind == "plugin" { &index.plugins } else { &index.skins };
    let item = market_item(items, id).ok_or_else(|| format!("市场中不存在 {id}"))?;
    let version = latest_market_version(item).ok_or_else(|| format!("市场条目 {id} 没有 latest 版本"))?;
    let (key, fingerprint) = package_key(item, version, &publishers)?;
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(120))
        .user_agent(concat!("SideWeave/", env!("CARGO_PKG_VERSION")))
        .build().map_err(|error| error.to_string())?;
    let release = &version.release;
    let zip = get_asset(app, &client, &release.url).await?;
    let signature = get_asset(app, &client, &release.signature_url).await?;
    let checksum = get_asset(app, &client, &release.checksum_url).await?;
    let expected = parse_checksum(&checksum)?;
    let digest = sha2::Sha256::digest(&zip);
    let actual: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    if actual != expected {
        return Err("安装包 SHA256 校验失败，已拒绝安装".into());
    }
    verify_minisign(&zip, &signature, &key.public_key)?;
    let manifest = read_package_manifest(&zip)?;
    let expected_type = if kind == "skin" { "skin" } else { item.package_type.as_str() };
    if manifest.id != item.id || manifest.version != version.version
        || manifest.package_type != expected_type
        || manifest.publisher != item.publisher || manifest.key_id != version.key_id
    {
        return Err("签名包内 manifest 与市场目录的 id、版本、类型或发布者密钥不匹配".into());
    }
    let actual_capabilities: std::collections::BTreeSet<_> = manifest.permissions.iter().map(String::as_str).collect();
    let declared_capabilities: std::collections::BTreeSet<_> = item.capabilities.iter().map(String::as_str).collect();
    if actual_capabilities != declared_capabilities {
        return Err("包内 capabilities 与市场目录声明不匹配".into());
    }
    let native = matches!(manifest.runtime, Some(PluginRuntime::ExternalProcess | PluginRuntime::DynamicLibrary));
    if kind == "plugin" && native != (item.package_type == "native") {
        return Err("包内 runtime 与市场目录 type 不匹配".into());
    }
    if native && !manifest.requires_trust {
        return Err("native 插件 manifest 必须声明 requires_trust: true".into());
    }
    install_from_bytes_with_policy(app, kind, &zip, true, before_replace).await?;
    if native {
        remember_trusted_key(app, &item.publisher, &key.id, &fingerprint)?;
    }
    Ok(())
}

/// 从本地 zip 字节安装（市场下载与"本地导入"共用的安装路径）。
/// 解压到临时目录 → 校验 manifest → 原子改名到最终目录。
async fn install_from_bytes_with_policy<R: Runtime>(
    app: &AppHandle<R>,
    kind: &str,
    bytes: &[u8],
    allow_trusted_external: bool,
    before_replace: impl FnOnce(&Manifest) -> Result<(), String>,
) -> Result<(), String> {
    if kind != "plugin" && kind != "skin" {
        return Err("未知的安装类型".into());
    }
    let data = paths::app_data_dir(app);
    let base = if kind == "plugin" {
        user_plugins_dir(&data)
    } else {
        user_skins_dir(&data)
    };
    std::fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    let tmp = base.join(format!(".import-{}", uuid::Uuid::new_v4()));

    let validate = || -> Result<Manifest, String> {
        unzip_into(bytes, &tmp)?;
        let Some(manifest) = read_manifest(&tmp) else {
            return Err("安装包缺少 manifest.json，已放弃安装".into());
        };
        if !valid_id(&manifest.id) {
            return Err(format!("manifest id 非法: {}", manifest.id));
        }
        if kind == "plugin" {
            shared_manifest(&manifest)?;
        }
        if kind == "skin" {
            if let Some(defaults) = manifest.appearance_defaults.as_ref() {
                defaults.validate()?;
            }
            if let Some(ui) = manifest.skin_ui.as_ref() {
                validate_skin_ui(&tmp, ui)?;
            } else if !tmp.join("skin.css").is_file() {
                return Err("皮肤包必须包含 skin.css 或完整的 skinUi 定义".into());
            }
        }
        if kind == "plugin" {
            if manifest.runtime.is_none() {
                return Err("插件 manifest 缺少 runtime，旧内置激活包不再支持".into());
            }
            let asset_root = manifest
                .ui
                .as_ref()
                .map(|ui| ui.asset_root.as_str())
                .filter(|root| !root.is_empty())
                .unwrap_or(".");
            if asset_root != "."
                && (!safe_relative_path(asset_root) || !tmp.join(asset_root).is_dir())
            {
                return Err("安装包插件 UI 资源根目录无效或不存在".into());
            }
            if matches!(
                manifest.runtime,
                Some(PluginRuntime::ExternalProcess | PluginRuntime::DynamicLibrary)
            ) {
                if !allow_trusted_external {
                    return Err("native 插件包必须来自受信任的本地导入或已验证签名的市场下载".into());
                }
                if asset_root == "." {
                    return Err("native 插件必须将 ui.assetRoot 设置为独立目录".into());
                }
                let Some(backend) = manifest.backend.as_deref() else {
                    return Err("native 插件缺少 backend 文件路径".into());
                };
                if !safe_relative_path(backend) || !tmp.join(backend).is_file() {
                    return Err("native 插件 backend 路径无效或文件不存在".into());
                }
            } else if manifest.runtime == Some(PluginRuntime::InProcess) {
                return Err("独立插件包不支持 in_process runtime，请使用 dynamic_library、external_process 或 web_sandbox".into());
            }
            let Some(entry) = plugin_ui_entry(&manifest) else {
                return Err("安装包缺少插件详情页入口，已放弃安装".into());
            };
            let entry_path = tmp.join(entry);
            let entry_inside_asset_root = std::fs::canonicalize(&entry_path)
                .ok()
                .zip(std::fs::canonicalize(tmp.join(asset_root)).ok())
                .is_some_and(|(entry, root)| entry.starts_with(root));
            if !safe_relative_path(entry) || !entry_path.is_file() || !entry_inside_asset_root {
                return Err("安装包插件详情页入口无效，已放弃安装".into());
            }
        }
        Ok(manifest)
    };
    let manifest = match validate() {
        Ok(m) => m,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(e);
        }
    };
    if let Err(error) = before_replace(&manifest) {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(error);
    }

    let dest = base.join(&manifest.id);
    if dest.exists() {
        std::fs::remove_dir_all(&dest).map_err(|e| e.to_string())?;
    }
    if std::fs::rename(&tmp, &dest).is_err() {
        // 跨卷兜底：复制目录
        copy_dir_all(&tmp, &dest)?;
        let _ = std::fs::remove_dir_all(&tmp);
    }
    if allow_trusted_external
        && matches!(
            manifest.runtime,
            Some(PluginRuntime::ExternalProcess | PluginRuntime::DynamicLibrary)
        )
    {
        std::fs::write(dest.join(TRUSTED_LOCAL_MARKER), b"trusted-local\n")
            .map_err(|error| format!("写入本地授信标记失败: {error}"))?;
    }
    Ok(())
}

fn copy_dir_all(src: &std::path::Path, dest: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    for entry in std::fs::read_dir(src).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let to = dest.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir_all(&entry.path(), &to)?;
        } else {
            std::fs::copy(entry.path(), to).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// 本地离线安装：弹出文件框选 zip（不经过市场，便于私有插件分发）。
/// 命令封装在 lib.rs（安装后需刷新轨道）。
pub async fn plugin_import_local<F>(
    app: AppHandle,
    kind: String,
    before_install: F,
) -> Result<(), String>
where
    F: FnOnce(&str) -> Result<(), String>,
{
    let file = crate::with_detail_not_always_on_top(&app, || {
        let mut dialog = app.dialog().file();
        if let Some(window) = crate::detail_window(&app) {
            dialog = dialog.set_parent(&window);
        }
        dialog.add_filter("插件包", &["zip"]).blocking_pick_file()
    });
    let Some(file) = file else {
        return Ok(());
    };
    let path = file.into_path().map_err(|e| format!("路径无效: {e}"))?;
    let bytes = std::fs::read(&path).map_err(|e| format!("读取文件失败: {e}"))?;
    install_from_bytes_with_policy(&app, &kind, &bytes, true, |manifest| {
        if kind == "plugin" {
            before_install(&manifest.id)?;
        }
        Ok(())
    })
    .await
}

pub async fn uninstall<R: Runtime>(
    app: &tauri::AppHandle<R>,
    kind: &str,
    id: &str,
) -> Result<(), String> {
    if kind != "plugin" && kind != "skin" {
        return Err("未知的卸载类型".into());
    }
    if !valid_id(id) {
        return Err("非法的 id".into());
    }
    let data = paths::app_data_dir(app);
    let dir = if kind == "plugin" {
        user_plugins_dir(&data).join(id)
    } else {
        user_skins_dir(&data).join(id)
    };
    let mut remove_error = None;
    if dir.exists() {
        if let Err(error) = std::fs::remove_dir_all(&dir) {
            remove_error = Some(error.to_string());
        }
    }
    // 兼容旧版本安装中断留下的 .import-* 目录，按卸载动作一并清理。
    let staging_root = if kind == "plugin" {
        user_plugins_dir(&data)
    } else {
        user_skins_dir(&data)
    };
    if let Ok(entries) = std::fs::read_dir(staging_root) {
        for entry in entries.flatten() {
            let path = entry.path();
            let belongs_to_plugin = read_manifest(&path).is_some_and(|manifest| manifest.id == id);
            if is_staging_dir(&path) && path.is_dir() && belongs_to_plugin {
                if let Err(error) = std::fs::remove_dir_all(path) {
                    remove_error.get_or_insert_with(|| error.to_string());
                }
            }
        }
    }
    if let Some(error) = remove_error {
        return Err(error);
    }
    // 原生插件卸载后的轻量清理：收起其悬浮窗（轨道入口已移除，窗体留在屏幕会不可控）
    if kind == "plugin" {
        let label = match id {
            "time" => Some("clock"),
            "thiefbook" => Some("reader"),
            _ => None,
        };
        if let Some(label) = label {
            if let Some(win) = app.get_webview_window(label) {
                let _ = win.hide();
            }
        }
    }
    Ok(())
}

// ---------- 用户插件运行时 ----------

const STORE_FILE: &str = "store.json";

fn store_path(app_data: &Path, plugin_id: &str) -> PathBuf {
    app_data
        .join("plugin-data")
        .join(plugin_id)
        .join(STORE_FILE)
}

pub fn store_get<R: Runtime>(
    app: &tauri::AppHandle<R>,
    plugin_id: &str,
    key: &str,
) -> Option<String> {
    if !valid_id(plugin_id) || !valid_id(key) {
        return None;
    }
    paths::load_json(&store_path(&paths::app_data_dir(app), plugin_id))
        .and_then(|v| v.get(key).and_then(|x| x.as_str().map(str::to_string)))
}

pub fn store_set<R: Runtime>(
    app: &tauri::AppHandle<R>,
    plugin_id: &str,
    key: &str,
    value: &str,
) -> Result<(), String> {
    if !valid_id(plugin_id) || !valid_id(key) {
        return Err("非法的存储键".into());
    }
    if value.len() > 256 * 1024 {
        return Err("存储值过大（>256KB）".into());
    }
    let path = store_path(&paths::app_data_dir(app), plugin_id);
    let mut obj = paths::load_json(&path).unwrap_or_else(|| serde_json::json!({}));
    if !obj.is_object() {
        obj = serde_json::json!({});
    }
    obj[key] = serde_json::Value::String(value.to_string());
    paths::save_json_atomic(&path, &obj).map_err(|e| e.to_string())
}

pub fn store_remove<R: Runtime>(
    app: &tauri::AppHandle<R>,
    plugin_id: &str,
    key: &str,
) -> Result<(), String> {
    if !valid_id(plugin_id) || !valid_id(key) {
        return Err("非法的存储键".into());
    }
    let path = store_path(&paths::app_data_dir(app), plugin_id);
    let mut obj = paths::load_json(&path).unwrap_or_else(|| serde_json::json!({}));
    if let Some(o) = obj.as_object_mut() {
        o.remove(key);
    }
    paths::save_json_atomic(&path, &obj).map_err(|e| e.to_string())
}

fn plugin_resource_path(uri_path: &str) -> Result<(String, String), String> {
    let path = uri_path.trim_start_matches('/');
    let (plugin_id, encoded_resource) = path
        .split_once('/')
        .ok_or_else(|| "插件资源 URL 缺少资源路径".to_string())?;
    if !valid_id(plugin_id) || encoded_resource.is_empty() {
        return Err("插件资源 URL 无效".into());
    }
    let resource = percent_encoding::percent_decode_str(encoded_resource)
        .decode_utf8()
        .map_err(|_| "插件资源路径不是有效 UTF-8".to_string())?
        .into_owned();
    if !safe_relative_path(&resource) {
        return Err("插件资源路径无效".into());
    }
    Ok((plugin_id.to_string(), resource))
}

fn resource_content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("bmp") => "image/bmp",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("avif") => "image/avif",
        Some("ico") => "image/x-icon",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        Some("otf") => "font/otf",
        Some("eot") => "application/vnd.ms-fontobject",
        Some("wasm") => "application/wasm",
        Some("mp3") => "audio/mpeg",
        Some("wav") => "audio/wav",
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
        _ => "application/octet-stream",
    }
}

fn plugin_resource_response_error(status: StatusCode, message: &str) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .body(message.as_bytes().to_vec())
        .expect("static plugin resource error response is valid")
}

const SKIN_BRIDGE_JS: &str = r#"(function () {
  if (window.parent === window || window.sideweaveSkin || window.agileTilesSkin) return;
  var skinId = window.__sideweaveSkinId || window.__agileTilesSkinId || '';
  var sequence = 0;
  var pending = new Map();
  var context = null;
  var contextListeners = new Set();
  var eventListeners = new Map();
  function send(message) {
    window.parent.postMessage(Object.assign({ __sideweaveSkin: true, __agileTilesSkin: true, skin_id: skinId }, message), '*');
  }
  function call(action, payload, timeoutMs) {
    return new Promise(function (resolve, reject) {
      var id = ++sequence;
      var timer = window.setTimeout(function () {
        pending.delete(id);
        reject(new Error('SideWeave UI request timed out'));
      }, timeoutMs || 8000);
      pending.set(id, {
        resolve: function (value) { window.clearTimeout(timer); resolve(value); },
        reject: function (error) { window.clearTimeout(timer); reject(error); }
      });
      send({ type: 'request', id: id, action: action, payload: payload || {} });
    });
  }
  window.addEventListener('message', function (event) {
    var message = event.data;
    if (event.source !== window.parent || !message || (message.__sideweaveSkin !== true && message.__agileTilesSkin !== true) || message.skin_id !== skinId) return;
    if (message.type === 'context') {
      context = message.context || null;
      var appearance = context && context.settings && context.settings.appearance || {};
      var surface = context && context.surface || 'detail';
      var rawPanelOpacity = surface === 'sidebar'
        ? appearance.sidebar_panel_opacity
        : surface === 'control'
          ? appearance.control_panel_opacity
          : appearance.detail_panel_opacity;
      var rawImageOpacity = surface === 'sidebar'
        ? appearance.sidebar_image_opacity
        : appearance.detail_image_opacity;
      var panelOpacity = typeof rawPanelOpacity === 'number' ? Math.max(0, Math.min(1, rawPanelOpacity)) : 1;
      var imageOpacity = typeof rawImageOpacity === 'number' ? Math.max(0, Math.min(1, rawImageOpacity)) : 1;
      var componentOpacity = typeof appearance.component_opacity === 'number'
        ? Math.max(0, Math.min(1, appearance.component_opacity))
        : 1;
      document.documentElement.style.setProperty('--skin-panel-opacity', String(panelOpacity));
      document.documentElement.style.setProperty('--skin-image-opacity', String(imageOpacity));
      document.documentElement.style.setProperty('--skin-background-opacity', String(imageOpacity));
      document.documentElement.style.setProperty('--skin-component-opacity', String(componentOpacity));
      contextListeners.forEach(function (handler) { handler(context); });
      return;
    }
    if (message.type === 'event') {
      (eventListeners.get(message.name) || []).slice().forEach(function (handler) { handler(message.payload); });
      return;
    }
    if (message.type === 'reply' && pending.has(message.id)) {
      var request = pending.get(message.id);
      pending.delete(message.id);
      if (message.error) request.reject(new Error(message.error));
      else request.resolve(message.result);
    }
  });
  var skinApi = Object.freeze({
    apiVersion: 1,
    id: skinId,
    call: call,
    getContext: function () { return context; },
    onContext: function (handler) {
      contextListeners.add(handler);
      if (context) handler(context);
      return function () { contextListeners.delete(handler); };
    },
    onEvent: function (name, handler) {
      if (!eventListeners.has(name)) eventListeners.set(name, new Set());
      eventListeners.get(name).add(handler);
      return function () {
        var listeners = eventListeners.get(name);
        if (!listeners) return;
        listeners.delete(handler);
        if (listeners.size === 0) eventListeners.delete(name);
      };
    }
  });
  window.sideweaveSkin = skinApi;
  window.agileTilesSkin = skinApi;
  function invokeWindowAction(action, payload) {
    call(action, payload || {}).catch(function () {});
  }
  function inDragRegion(target) {
    for (var node = target; node && node !== document.documentElement; node = node.parentElement) {
      if (node.hasAttribute && node.hasAttribute('data-tauri-drag-region')) {
        return node.getAttribute('data-tauri-drag-region') !== 'false';
      }
      if (node.classList && node.classList.contains('drag-region')) return true;
    }
    return false;
  }
  function installResizeHandles() {
    var surface = document.body && document.body.getAttribute('data-surface');
    if ((surface !== 'detail' && surface !== 'control') || document.getElementById('__sideweave_resize_style')) return;
    var style = document.createElement('style');
    style.id = '__sideweave_resize_style';
    style.textContent = [
      '[data-sideweave_resize]{position:fixed;z-index:2147483647;pointer-events:auto;background:transparent;touch-action:none}',
      '[data-sideweave_resize="North"]{top:0;left:12px;right:12px;height:6px;cursor:n-resize}',
      '[data-sideweave_resize="South"]{bottom:0;left:12px;right:12px;height:6px;cursor:s-resize}',
      '[data-sideweave_resize="East"]{top:12px;right:0;bottom:12px;width:6px;cursor:e-resize}',
      '[data-sideweave_resize="West"]{top:12px;left:0;bottom:12px;width:6px;cursor:w-resize}',
      '[data-sideweave_resize="NorthEast"]{top:0;right:0;width:14px;height:14px;cursor:ne-resize}',
      '[data-sideweave_resize="NorthWest"]{top:0;left:0;width:14px;height:14px;cursor:nw-resize}',
      '[data-sideweave_resize="SouthEast"]{right:0;bottom:0;width:14px;height:14px;cursor:se-resize}',
      '[data-sideweave_resize="SouthWest"]{left:0;bottom:0;width:14px;height:14px;cursor:sw-resize}'
    ].join('');
    document.head.appendChild(style);
    ['North', 'South', 'East', 'West', 'NorthEast', 'NorthWest', 'SouthEast', 'SouthWest'].forEach(function (direction) {
      var handle = document.createElement('div');
      handle.setAttribute('data-sideweave_resize', direction);
      handle.setAttribute('aria-hidden', 'true');
      document.body.appendChild(handle);
    });
  }
  document.addEventListener('mousedown', function (event) {
    if (event.button !== 0) return;
    var target = event.target;
    if (!target || typeof target.closest !== 'function') return;
    var handle = target.closest('[data-sideweave_resize]');
    if (handle) {
      event.preventDefault();
      event.stopPropagation();
      invokeWindowAction('window.startResizeDragging', { direction: handle.getAttribute('data-sideweave_resize') });
      return;
    }
    if (inDragRegion(target)) invokeWindowAction('window.startDragging');
  }, true);
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', installResizeHandles, { once: true });
  } else {
    installResizeHandles();
  }
  send({ type: 'ready' });
})();"#;

fn inject_skin_bridge(html: String, skin_id: &str) -> Result<Vec<u8>, String> {
    let skin_id =
        serde_json::to_string(skin_id).map_err(|_| "Could not encode skin id".to_string())?;
    let bridge = format!(
        "<script>window.__sideweaveSkinId={};window.__agileTilesSkinId=window.__sideweaveSkinId;</script><script>{}</script>",
        skin_id, SKIN_BRIDGE_JS
    );
    let lower = html.to_ascii_lowercase();
    let insertion_point = lower
        .find("<head")
        .and_then(|index| lower[index..].find('>').map(|offset| index + offset + 1))
        .or_else(|| {
            lower
                .find("<!doctype")
                .and_then(|index| lower[index..].find('>').map(|offset| index + offset + 1))
        })
        .unwrap_or(0);
    let mut html = html;
    html.insert_str(insertion_point, &bridge);
    Ok(html.into_bytes())
}

pub fn plugin_resource_response<R: Runtime>(
    app: &tauri::AppHandle<R>,
    request: Request<Vec<u8>>,
) -> Response<Vec<u8>> {
    if request.uri().path().starts_with("/@skin/") {
        return skin_resource_response(app, request);
    }
    if request.method() != tauri::http::Method::GET && request.method() != tauri::http::Method::HEAD
    {
        return plugin_resource_response_error(
            StatusCode::METHOD_NOT_ALLOWED,
            "只允许读取插件资源",
        );
    }
    let (plugin_id, resource) = match plugin_resource_path(request.uri().path()) {
        Ok(value) => value,
        Err(message) => return plugin_resource_response_error(StatusCode::BAD_REQUEST, &message),
    };
    let data_dir = paths::app_data_dir(app);
    let plugin_dir = user_plugins_dir(&data_dir).join(&plugin_id);
    let Some(manifest) = read_manifest(&plugin_dir) else {
        return plugin_resource_response_error(StatusCode::NOT_FOUND, "插件未安装");
    };
    if manifest.runtime == Some(PluginRuntime::InProcess) {
        return plugin_resource_response_error(
            StatusCode::FORBIDDEN,
            "插件运行时不允许读取 Web 资源",
        );
    }
    let asset_root = manifest
        .ui
        .as_ref()
        .map(|ui| ui.asset_root.as_str())
        .filter(|root| !root.is_empty())
        .unwrap_or(".");
    if !safe_relative_path(asset_root) && asset_root != "." {
        return plugin_resource_response_error(StatusCode::FORBIDDEN, "插件资源根目录无效");
    }
    let asset_root_path = plugin_dir.join(asset_root);
    let candidate = plugin_dir.join(&resource);
    let Ok(asset_root_canonical) = std::fs::canonicalize(&asset_root_path) else {
        return plugin_resource_response_error(StatusCode::NOT_FOUND, "插件资源根目录不存在");
    };
    let Ok(candidate_canonical) = std::fs::canonicalize(&candidate) else {
        return plugin_resource_response_error(StatusCode::NOT_FOUND, "插件资源不存在");
    };
    if !candidate_canonical.starts_with(&asset_root_canonical)
        || matches!(
            resource.as_str(),
            "manifest.json" | "store.json" | TRUSTED_LOCAL_MARKER
        )
        || manifest.backend.as_deref() == Some(resource.as_str())
    {
        return plugin_resource_response_error(StatusCode::FORBIDDEN, "插件资源访问被拒绝");
    }
    let Ok(mut bytes) = std::fs::read(&candidate_canonical) else {
        return plugin_resource_response_error(StatusCode::NOT_FOUND, "插件资源读取失败");
    };
    if candidate_canonical
        .extension()
        .and_then(|extension| extension.to_str())
        == Some("html")
    {
        let Ok(html) = String::from_utf8(bytes) else {
            return plugin_resource_response_error(
                StatusCode::BAD_REQUEST,
                "插件 HTML 不是有效 UTF-8",
            );
        };
        let Ok(plugin_id_json) = serde_json::to_string(&manifest.id) else {
            return plugin_resource_response_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "插件 ID 序列化失败",
            );
        };
        bytes = format!(
            "<script>window.__agileTilesPluginId = {};</script>\n<script>{}</script>\n{}",
            plugin_id_json,
            crate::plugins::user_bridge::BRIDGE_JS,
            html
        )
        .into_bytes();
    }
    Response::builder()
        .status(StatusCode::OK)
        .header(
            header::CONTENT_TYPE,
            resource_content_type(&candidate_canonical),
        )
        .header(header::CACHE_CONTROL, "no-cache")
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .body(bytes)
        .expect("static plugin resource response is valid")
}

fn skin_resource_path(uri_path: &str) -> Result<(String, String), String> {
    let path = uri_path
        .strip_prefix("/@skin/")
        .ok_or_else(|| "skin resource URL is invalid".to_string())?;
    let (skin_id, encoded_resource) = path
        .split_once('/')
        .ok_or_else(|| "skin resource URL is missing a resource path".to_string())?;
    if !valid_id(skin_id) || encoded_resource.is_empty() {
        return Err("skin resource URL is invalid".into());
    }
    let resource = percent_encoding::percent_decode_str(encoded_resource)
        .decode_utf8()
        .map_err(|_| "skin resource path is not valid UTF-8".to_string())?
        .into_owned();
    if !safe_relative_path(&resource) {
        return Err("skin resource path is invalid".into());
    }
    Ok((skin_id.to_string(), resource))
}

fn validate_skin_ui(dir: &Path, ui: &SkinUi) -> Result<(), String> {
    let asset_root = if ui.asset_root.is_empty() {
        "."
    } else {
        ui.asset_root.as_str()
    };
    if asset_root != "." && !safe_relative_path(asset_root) {
        return Err("skin UI assetRoot must be a safe relative directory".into());
    }
    let root = dir.join(asset_root);
    let canonical_root =
        std::fs::canonicalize(&root).map_err(|_| "skin UI assetRoot does not exist".to_string())?;
    for (surface, entry) in [
        ("sidebar", ui.sidebar.as_str()),
        ("detail", ui.detail.as_str()),
        ("control", ui.control.as_str()),
        ("preview", ui.preview.as_str()),
    ] {
        if !safe_relative_path(entry) {
            return Err(format!(
                "skin UI {surface} entry must be a safe relative path"
            ));
        }
        let entry_path = root.join(entry);
        let canonical_entry = std::fs::canonicalize(&entry_path)
            .map_err(|_| format!("skin UI {surface} entry does not exist"))?;
        if !canonical_entry.starts_with(&canonical_root) || !canonical_entry.is_file() {
            return Err(format!(
                "skin UI {surface} entry is outside assetRoot or is not a file"
            ));
        }
    }
    Ok(())
}

fn skin_resource_response<R: Runtime>(
    app: &tauri::AppHandle<R>,
    request: Request<Vec<u8>>,
) -> Response<Vec<u8>> {
    if request.method() != tauri::http::Method::GET && request.method() != tauri::http::Method::HEAD
    {
        return plugin_resource_response_error(
            StatusCode::METHOD_NOT_ALLOWED,
            "Only GET and HEAD are allowed for skin resources",
        );
    }
    let (skin_id, resource) = match skin_resource_path(request.uri().path()) {
        Ok(value) => value,
        Err(message) => return plugin_resource_response_error(StatusCode::BAD_REQUEST, &message),
    };
    let data_dir = paths::app_data_dir(app);
    let skin_dir = user_skins_dir(&data_dir).join(&skin_id);
    let Some(manifest) = read_manifest(&skin_dir) else {
        return plugin_resource_response_error(StatusCode::NOT_FOUND, "Skin is not installed");
    };
    let root_resource = resource.strip_prefix("@root/");
    let Some(ui) = manifest.skin_ui.as_ref() else {
        if root_resource.is_none() {
            return plugin_resource_response_error(StatusCode::NOT_FOUND, "Skin has no custom UI");
        }
        // Legacy CSS-only skins may load their packaged image/font assets from the package root.
        // The request still goes through the canonical path and private-file checks below.
        if !skin_dir.join("skin.css").is_file() {
            return plugin_resource_response_error(StatusCode::NOT_FOUND, "Skin has no custom UI");
        }
        let relative = root_resource.unwrap_or_default();
        return read_skin_resource(&skin_dir, &skin_dir, relative, &manifest.id);
    };
    if let Err(error) = validate_skin_ui(&skin_dir, ui) {
        return plugin_resource_response_error(StatusCode::FORBIDDEN, &error);
    }
    if matches!(
        resource.as_str(),
        "manifest.json" | "store.json" | TRUSTED_LOCAL_MARKER
    ) {
        return plugin_resource_response_error(StatusCode::FORBIDDEN, "Skin metadata is private");
    }
    let asset_root = if ui.asset_root.is_empty() {
        "."
    } else {
        ui.asset_root.as_str()
    };
    let root_path = skin_dir.join(asset_root);
    let Ok(root) = std::fs::canonicalize(&root_path) else {
        return plugin_resource_response_error(
            StatusCode::NOT_FOUND,
            "Skin assetRoot does not exist",
        );
    };
    if let Some(root_relative) = root_resource {
        return read_skin_resource(&skin_dir, &skin_dir, root_relative, &manifest.id);
    }
    read_skin_resource(&skin_dir, &root, &resource, &manifest.id)
}

fn read_skin_resource(
    skin_dir: &Path,
    allowed_root: &Path,
    relative: &str,
    skin_id: &str,
) -> Response<Vec<u8>> {
    let private_file = ["manifest.json", "store.json", TRUSTED_LOCAL_MARKER]
        .iter()
        .any(|name| relative.eq_ignore_ascii_case(name));
    if !safe_relative_path(relative) || private_file {
        return plugin_resource_response_error(
            StatusCode::FORBIDDEN,
            "Skin resource is private or invalid",
        );
    }
    let Ok(candidate) = std::fs::canonicalize(skin_dir.join(relative)) else {
        return plugin_resource_response_error(
            StatusCode::NOT_FOUND,
            "Skin resource does not exist",
        );
    };
    let Ok(allowed_root) = std::fs::canonicalize(allowed_root) else {
        return plugin_resource_response_error(
            StatusCode::NOT_FOUND,
            "Skin asset root does not exist",
        );
    };
    if !candidate.starts_with(&allowed_root) {
        return plugin_resource_response_error(
            StatusCode::FORBIDDEN,
            "Skin resource is outside its asset root",
        );
    }
    let Ok(mut bytes) = std::fs::read(&candidate) else {
        return plugin_resource_response_error(
            StatusCode::NOT_FOUND,
            "Could not read skin resource",
        );
    };
    if candidate
        .extension()
        .and_then(|extension| extension.to_str())
        == Some("html")
    {
        let Ok(html) = String::from_utf8(bytes) else {
            return plugin_resource_response_error(
                StatusCode::BAD_REQUEST,
                "Skin HTML is not valid UTF-8",
            );
        };
        let Ok(injected) = inject_skin_bridge(html, skin_id) else {
            return plugin_resource_response_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Could not prepare skin UI",
            );
        };
        bytes = injected;
    }
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, resource_content_type(&candidate))
        .header(header::CACHE_CONTROL, "no-cache")
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .body(bytes)
        .expect("static skin resource response is valid")
}

pub fn is_web_plugin(app_data: &Path, plugin_id: &str) -> bool {
    let Some(manifest) = read_manifest(&user_plugins_dir(app_data).join(plugin_id)) else {
        return false;
    };
    let Ok(shared) = shared_manifest(&manifest) else {
        return false;
    };
    shared.runtime == PluginRuntime::WebSandbox
}

// ---------- 用户皮肤 ----------

pub fn user_skin_css<R: Runtime>(app: &tauri::AppHandle<R>, id: &str) -> Result<String, String> {
    if !valid_id(id) {
        return Err("非法的 id".into());
    }
    let dir = user_skins_dir(&paths::app_data_dir(app)).join(id);
    let css = std::fs::read_to_string(dir.join("skin.css"))
        .map_err(|_| "皮肤未安装或缺少 skin.css".to_string())?;
    Ok(rewrite_skin_css_urls(id, &css))
}

fn rewrite_skin_css_urls(skin_id: &str, css: &str) -> String {
    let lowered = css.to_ascii_lowercase();
    let mut output = String::with_capacity(css.len());
    let mut cursor = 0;
    while let Some(offset) = lowered[cursor..].find("url(") {
        let start = cursor + offset;
        let value_start = start + 4;
        let Some(close) = css_url_close(css, value_start) else {
            break;
        };
        output.push_str(&css[cursor..value_start]);
        let raw = &css[value_start..close];
        output.push_str(&rewrite_skin_css_url(skin_id, raw));
        output.push(')');
        cursor = close + 1;
    }
    output.push_str(&css[cursor..]);
    output
}

fn css_url_close(css: &str, start: usize) -> Option<usize> {
    let mut quote = None;
    let mut escaped = false;
    let mut nested = 0usize;
    for (offset, ch) in css[start..].char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if let Some(current_quote) = quote {
            if ch == current_quote {
                quote = None;
            }
            continue;
        }
        if ch == '\'' || ch == '"' {
            quote = Some(ch);
        } else if ch == '(' {
            nested += 1;
        } else if ch == ')' {
            if nested == 0 {
                return Some(start + offset);
            }
            nested -= 1;
        }
    }
    None
}

fn rewrite_skin_css_url(skin_id: &str, raw: &str) -> String {
    let leading = raw.len() - raw.trim_start().len();
    let trailing = raw.len() - raw.trim_end().len();
    let trimmed = raw.trim();
    let quote = trimmed
        .chars()
        .next()
        .filter(|ch| *ch == '\'' || *ch == '"');
    let value = match quote {
        Some(quote) if trimmed.ends_with(quote) && trimmed.len() >= 2 => {
            &trimmed[1..trimmed.len() - 1]
        }
        Some(_) => return raw.to_string(),
        None => trimmed,
    };
    let lower = value.to_ascii_lowercase();
    if value.is_empty()
        || value.starts_with('/')
        || value.starts_with('#')
        || value.contains('\\')
        || value.contains(':')
        || value.contains('(')
        || lower.starts_with("data:")
        || lower.starts_with("blob:")
    {
        return raw.to_string();
    }
    let suffix_start = value
        .char_indices()
        .find_map(|(index, ch)| (ch == '?' || ch == '#').then_some(index))
        .unwrap_or(value.len());
    let (path, suffix) = value.split_at(suffix_start);
    let mut segments = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => return raw.to_string(),
            part => segments.push(part),
        }
    }
    if segments.is_empty() {
        return raw.to_string();
    }
    let encoded_path = segments
        .into_iter()
        .map(|segment| {
            percent_encoding::utf8_percent_encode(segment, percent_encoding::NON_ALPHANUMERIC)
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("/");
    let resolved =
        format!("http://agile-plugin.localhost/@skin/{skin_id}/@root/{encoded_path}{suffix}");
    let mut result = String::with_capacity(raw.len() + resolved.len());
    result.push_str(&raw[..leading]);
    if let Some(quote) = quote {
        result.push(quote);
        result.push_str(&resolved);
        result.push(quote);
    } else {
        result.push_str(&resolved);
    }
    if trailing > 0 {
        result.push_str(&raw[raw.len() - trailing..]);
    }
    result
}

pub fn user_skin_ui<R: Runtime>(
    app: &tauri::AppHandle<R>,
    id: &str,
) -> Result<Option<SkinUi>, String> {
    if !valid_id(id) {
        return Err("Invalid skin id".into());
    }
    let dir = user_skins_dir(&paths::app_data_dir(app)).join(id);
    let manifest = read_manifest(&dir).ok_or_else(|| "Skin is not installed".to_string())?;
    if let Some(ui) = manifest.skin_ui.as_ref() {
        validate_skin_ui(&dir, ui)?;
    }
    Ok(manifest.skin_ui)
}

pub fn web_plugin_call<R: Runtime>(
    app: &tauri::AppHandle<R>,
    request: PluginRequest,
) -> Result<PluginResponse, String> {
    request
        .validate_for(&request.plugin_id)
        .map_err(|error| error.to_string())?;
    if !matches!(request.kind, PluginRequestKind::Call) {
        return Err("web plugins only support call requests".into());
    }
    let plugin_id = request.plugin_id.to_string();
    let data_dir = paths::app_data_dir(app);
    let plugin_dir = user_plugins_dir(&data_dir).join(&plugin_id);
    let manifest = read_manifest(&plugin_dir).ok_or("插件未安装")?;
    let shared = shared_manifest(&manifest)?;
    if shared.runtime == PluginRuntime::WebSandbox {
        let method = request.method.as_deref().unwrap_or_default();
        if let Some(permission) = sideweave_plugin_api::permissions::for_host_method(method) {
            let denied_reason = if !sideweave_plugin_api::permissions::is_declared(
                &shared.permissions,
                permission,
            ) {
                Some(format!("插件未声明此权限：{permission}"))
            } else if !crate::plugin_permission_granted(app, &shared.id, permission) {
                Some(format!("此插件权限已在设置中关闭：{permission}"))
            } else {
                None
            };
            if let Some(message) = denied_reason {
                return Ok(PluginResponse {
                    request_id: request.request_id,
                    plugin_id: request.plugin_id,
                    ok: false,
                    result: serde_json::Value::Null,
                    error: Some(PluginError {
                        code: ErrorCode::PermissionDenied,
                        message,
                    }),
                });
            }
        }
        let result = match method {
            "host.storage.get" => {
                let key = request
                    .payload
                    .get("key")
                    .and_then(|value| value.as_str())
                    .ok_or("缺少存储键")?;
                serde_json::json!(store_get(app, &plugin_id, key))
            }
            "host.storage.set" => {
                let key = request
                    .payload
                    .get("key")
                    .and_then(|value| value.as_str())
                    .ok_or("缺少存储键")?;
                let value = request
                    .payload
                    .get("value")
                    .and_then(|value| value.as_str())
                    .ok_or("缺少存储值")?;
                store_set(app, &plugin_id, key, value)?;
                serde_json::Value::Null
            }
            "host.storage.remove" => {
                let key = request
                    .payload
                    .get("key")
                    .and_then(|value| value.as_str())
                    .ok_or("缺少存储键")?;
                store_remove(app, &plugin_id, key)?;
                serde_json::Value::Null
            }
            "host.system.open_external" => {
                let url = request
                    .payload
                    .get("url")
                    .and_then(|value| value.as_str())
                    .ok_or("缺少 URL")?;
                open_external_url(url)?;
                serde_json::Value::Null
            }
            _ => {
                return Ok(PluginResponse {
                    request_id: request.request_id,
                    plugin_id: request.plugin_id,
                    ok: false,
                    result: serde_json::Value::Null,
                    error: Some(PluginError {
                        code: ErrorCode::PermissionDenied,
                        message: format!("web plugin method is not allowed: {method}"),
                    }),
                })
            }
        };
        return Ok(PluginResponse {
            request_id: request.request_id,
            plugin_id: request.plugin_id,
            ok: true,
            result,
            error: None,
        });
    }
    Err("plugin is not a web sandbox plugin".into())
}

pub fn open_external_url(url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|_| "仅支持打开有效的 http(s) 链接".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || url.chars().any(char::is_control)
        || url.contains('"')
    {
        return Err("仅支持打开 http(s) 链接".into());
    }
    let url = parsed.as_str();
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std::process::Command::new("cmd")
            .args(["/C", "start", ""])
            .raw_arg(format!("\"{url}\""))
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|error| format!("打开浏览器失败: {error}"))?;
    }
    #[cfg(not(windows))]
    {
        return Err("仅支持 Windows 外部链接打开".into());
    }
    Ok(())
}

/// Convert installed package metadata into the shared runtime registry model.
pub fn runtime_plugin_manifests(app_data: &Path) -> Vec<(PluginManifest, PathBuf, String)> {
    let dir = user_plugins_dir(app_data);
    let mut manifests = Vec::new();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return manifests;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() || is_staging_dir(&path) {
            continue;
        }
        let Some(legacy) = read_manifest(&path) else {
            continue;
        };
        if RETIRED_PLUGIN_IDS.contains(&legacy.id.as_str()) {
            continue;
        }
        if legacy.runtime.is_none() {
            continue;
        }
        let Ok(manifest) = shared_manifest(&legacy) else {
            continue;
        };
        let source = match manifest.runtime {
            PluginRuntime::DynamicLibrary => "installed-dylib",
            PluginRuntime::ExternalProcess => "installed-external",
            PluginRuntime::InProcess => "rejected-in-process",
            PluginRuntime::WebSandbox => "installed-web",
        };
        manifests.push((manifest, path, source.into()));
    }
    let mut deduped = HashMap::<String, (PluginManifest, PathBuf, String)>::new();
    for (manifest, path, source) in manifests {
        let id = manifest.id.to_string();
        let replace = deduped
            .get(&id)
            .map(|current| {
                compare_versions(&manifest.version, &current.0.version) == Ordering::Greater
            })
            .unwrap_or(true);
        if replace {
            deduped.insert(id, (manifest, path, source));
        }
    }
    let mut manifests: Vec<_> = deduped.into_values().collect();
    manifests.sort_by(|left, right| left.0.id.as_str().cmp(right.0.id.as_str()));
    manifests
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_prerelease_versions_before_stable_versions() {
        assert_eq!(
            compare_versions("1.2.3-beta.10", "1.2.3-beta.2"),
            Ordering::Greater
        );
        assert_eq!(
            compare_versions("1.2.3", "1.2.3-beta.10"),
            Ordering::Greater
        );
        assert_eq!(compare_versions("1.2.3-alpha", "1.2.3"), Ordering::Less);
    }

    fn external_manifest(backend: &str) -> PluginManifest {
        PluginManifest {
            manifest_version: MANIFEST_VERSION,
            api_version: API_VERSION,
            id: PluginId::new("test-plugin").expect("test plugin id is valid"),
            name: "Test Plugin".into(),
            icon: "puzzle".into(),
            version: "1.0.0".into(),
            description: String::new(),
            author: String::new(),
            runtime: PluginRuntime::ExternalProcess,
            backend: Some(backend.into()),
            trust: PluginTrust::TrustedLocal,
            auto_start: false,
            permissions: Vec::new(),
            required_permissions: Vec::new(),
            ui: None,
            hover_preview: None,
            minimum_host_version: None,
        }
    }

    #[test]
    fn backend_path_stays_inside_plugin_package() {
        let root =
            std::env::temp_dir().join(format!("sideweave_market-test-{}", uuid::Uuid::new_v4()));
        let backend = root.join("backend").join("plugin.exe");
        std::fs::create_dir_all(backend.parent().expect("backend parent exists"))
            .expect("create test backend directory");
        std::fs::write(&backend, b"test").expect("write test backend");

        let manifest = external_manifest("backend/plugin.exe");
        assert_eq!(
            plugin_backend_path(&root, &manifest),
            Some(backend.canonicalize().unwrap())
        );
        assert!(plugin_backend_path(&root, &external_manifest("../plugin.exe")).is_none());

        std::fs::remove_dir_all(root).expect("remove test package");
    }
}
