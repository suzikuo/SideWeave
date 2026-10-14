//! 自适应性能与资源策略调度中心 (PerformanceManager)
//!
//! 职责划分：
//! 1. 提供统一定义的运行模式档位（PerformanceProfile）：Eco、Balanced、Performance、Auto。
//! 2. 结合 Windows 物理内存与系统可用内存，计算当前实际生效的策略（PerformancePolicy）。
//! 3. 业务代码只面向 `PerformancePolicy` 决策（如 prewarm、keep_alive、hover_delay），杜绝散落判断。

use serde::{Deserialize, Serialize};
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PerformanceProfile {
    Eco,
    Balanced,
    Performance,
    Auto,
}

impl PerformanceProfile {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "eco" => Some(Self::Eco),
            "balanced" => Some(Self::Balanced),
            "performance" => Some(Self::Performance),
            "auto" => Some(Self::Auto),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Eco => "eco",
            Self::Balanced => "balanced",
            Self::Performance => "performance",
            Self::Auto => "auto",
        }
    }
}

impl Default for PerformanceProfile {
    fn default() -> Self {
        Self::Balanced
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerformancePolicy {
    /// 启动后是否在后台静默预热详情窗口
    pub detail_prewarm: bool,
    /// 详情窗口关闭时是否保活（只 hide 不 destroy）
    pub detail_keep_alive: bool,
    /// 启动后是否在后台静默预热悬浮预览窗口
    pub preview_prewarm: bool,
    /// 悬浮卡片关闭时是否保活（只 hide 不 destroy）
    pub preview_keep_alive: bool,
    /// 悬浮防抖响应时间（毫秒）
    pub hover_delay_ms: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemMemorySnapshot {
    pub total_phys_bytes: u64,
    pub avail_phys_bytes: u64,
    pub memory_load_pct: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerformanceStatusPayload {
    pub user_profile: PerformanceProfile,
    pub effective_profile: PerformanceProfile,
    pub policy: PerformancePolicy,
    pub memory: Option<SystemMemorySnapshot>,
}

#[cfg(windows)]
pub fn query_system_memory() -> Option<SystemMemorySnapshot> {
    use std::mem::size_of;
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

    let mut status = MEMORYSTATUSEX {
        dwLength: size_of::<MEMORYSTATUSEX>() as u32,
        dwMemoryLoad: 0,
        ullTotalPhys: 0,
        ullAvailPhys: 0,
        ullTotalPageFile: 0,
        ullAvailPageFile: 0,
        ullTotalVirtual: 0,
        ullAvailVirtual: 0,
        ullAvailExtendedVirtual: 0,
    };

    let ok = unsafe { GlobalMemoryStatusEx(&mut status) };
    if ok != 0 {
        Some(SystemMemorySnapshot {
            total_phys_bytes: status.ullTotalPhys,
            avail_phys_bytes: status.ullAvailPhys,
            memory_load_pct: status.dwMemoryLoad,
        })
    } else {
        None
    }
}

#[cfg(not(windows))]
pub fn query_system_memory() -> Option<SystemMemorySnapshot> {
    None
}

/// 自动根据系统内存情况评估有效 Profile
pub fn evaluate_auto_profile(mem: &SystemMemorySnapshot) -> PerformanceProfile {
    let four_gb = 4 * 1024 * 1024 * 1024;
    let two_gb = 2 * 1024 * 1024 * 1024;
    let twenty_pct = mem.total_phys_bytes / 5;
    let ten_pct = mem.total_phys_bytes / 10;

    let normal_avail_threshold = four_gb.max(twenty_pct);
    let warning_avail_threshold = two_gb.max(ten_pct);

    if mem.avail_phys_bytes >= normal_avail_threshold && mem.memory_load_pct < 75 {
        PerformanceProfile::Performance
    } else if mem.avail_phys_bytes >= warning_avail_threshold && mem.memory_load_pct < 88 {
        PerformanceProfile::Balanced
    } else {
        PerformanceProfile::Eco
    }
}

pub struct PerformanceManager {
    user_profile: Mutex<PerformanceProfile>,
}

impl PerformanceManager {
    pub fn new(initial_profile: PerformanceProfile) -> Self {
        Self {
            user_profile: Mutex::new(initial_profile),
        }
    }

    pub fn set_profile(&self, profile: PerformanceProfile) {
        *self.user_profile.lock().unwrap() = profile;
    }

    pub fn user_profile(&self) -> PerformanceProfile {
        *self.user_profile.lock().unwrap()
    }

    pub fn effective_profile(&self) -> PerformanceProfile {
        let profile = self.user_profile();
        if profile != PerformanceProfile::Auto {
            return profile;
        }

        if let Some(mem) = query_system_memory() {
            evaluate_auto_profile(&mem)
        } else {
            PerformanceProfile::Balanced
        }
    }

    pub fn effective_policy(&self, user_hover_delay: Option<u32>) -> PerformancePolicy {
        match self.effective_profile() {
            PerformanceProfile::Eco => PerformancePolicy {
                detail_prewarm: false,
                detail_keep_alive: false,
                preview_prewarm: false,
                preview_keep_alive: false,
                hover_delay_ms: 350,
            },
            PerformanceProfile::Balanced => PerformancePolicy {
                detail_prewarm: false,
                detail_keep_alive: false,
                preview_prewarm: false,
                preview_keep_alive: true,
                hover_delay_ms: user_hover_delay.unwrap_or(350),
            },
            PerformanceProfile::Performance => PerformancePolicy {
                detail_prewarm: true,
                detail_keep_alive: true,
                preview_prewarm: true,
                preview_keep_alive: true,
                hover_delay_ms: 200,
            },
            PerformanceProfile::Auto => unreachable!(),
        }
    }

    pub fn status_payload(&self, user_hover_delay: Option<u32>) -> PerformanceStatusPayload {
        let user_profile = self.user_profile();
        let memory = query_system_memory();
        let effective_profile = match user_profile {
            PerformanceProfile::Auto => memory
                .as_ref()
                .map(evaluate_auto_profile)
                .unwrap_or(PerformanceProfile::Balanced),
            other => other,
        };
        let policy = match effective_profile {
            PerformanceProfile::Eco => PerformancePolicy {
                detail_prewarm: false,
                detail_keep_alive: false,
                preview_prewarm: false,
                preview_keep_alive: false,
                hover_delay_ms: 350,
            },
            PerformanceProfile::Balanced => PerformancePolicy {
                detail_prewarm: false,
                detail_keep_alive: true,
                preview_prewarm: false,
                preview_keep_alive: true,
                hover_delay_ms: user_hover_delay.unwrap_or(350),
            },
            PerformanceProfile::Performance => PerformancePolicy {
                detail_prewarm: true,
                detail_keep_alive: true,
                preview_prewarm: true,
                preview_keep_alive: true,
                hover_delay_ms: 200,
            },
            PerformanceProfile::Auto => unreachable!(),
        };

        PerformanceStatusPayload {
            user_profile,
            effective_profile,
            policy,
            memory,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_profile_parsing_and_conversion() {
        assert_eq!(
            PerformanceProfile::parse("eco"),
            Some(PerformanceProfile::Eco)
        );
        assert_eq!(
            PerformanceProfile::parse("balanced"),
            Some(PerformanceProfile::Balanced)
        );
        assert_eq!(
            PerformanceProfile::parse("performance"),
            Some(PerformanceProfile::Performance)
        );
        assert_eq!(
            PerformanceProfile::parse("auto"),
            Some(PerformanceProfile::Auto)
        );
        assert_eq!(PerformanceProfile::parse("invalid"), None);
    }

    #[test]
    fn test_auto_evaluation_thresholds() {
        // 16GB 机器，剩余 8GB，负荷 50% -> Performance
        let normal_mem = SystemMemorySnapshot {
            total_phys_bytes: 16 * 1024 * 1024 * 1024,
            avail_phys_bytes: 8 * 1024 * 1024 * 1024,
            memory_load_pct: 50,
        };
        assert_eq!(
            evaluate_auto_profile(&normal_mem),
            PerformanceProfile::Performance
        );

        // 16GB 机器，剩余 3GB，负荷 81% -> Balanced
        let warning_mem = SystemMemorySnapshot {
            total_phys_bytes: 16 * 1024 * 1024 * 1024,
            avail_phys_bytes: 3 * 1024 * 1024 * 1024,
            memory_load_pct: 81,
        };
        assert_eq!(
            evaluate_auto_profile(&warning_mem),
            PerformanceProfile::Balanced
        );

        // 16GB 机器，剩余 1GB，负荷 93% -> Eco
        let critical_mem = SystemMemorySnapshot {
            total_phys_bytes: 16 * 1024 * 1024 * 1024,
            avail_phys_bytes: 1 * 1024 * 1024 * 1024,
            memory_load_pct: 93,
        };
        assert_eq!(
            evaluate_auto_profile(&critical_mem),
            PerformanceProfile::Eco
        );
    }

    #[test]
    fn test_effective_policy_mapping() {
        let mgr = PerformanceManager::new(PerformanceProfile::Performance);
        let policy = mgr.effective_policy(Some(350));
        assert!(policy.detail_prewarm);
        assert!(policy.detail_keep_alive);
        assert!(policy.preview_prewarm);
        assert!(policy.preview_keep_alive);
        assert_eq!(policy.hover_delay_ms, 200);

        mgr.set_profile(PerformanceProfile::Balanced);
        let balanced_policy = mgr.effective_policy(Some(350));
        assert!(!balanced_policy.detail_prewarm);
        assert!(!balanced_policy.detail_keep_alive);
        assert!(!balanced_policy.preview_prewarm);
        assert!(balanced_policy.preview_keep_alive);
        assert_eq!(balanced_policy.hover_delay_ms, 350);

        mgr.set_profile(PerformanceProfile::Eco);
        let eco_policy = mgr.effective_policy(Some(350));
        assert!(!eco_policy.detail_prewarm);
        assert!(!eco_policy.detail_keep_alive);
        assert!(!eco_policy.preview_prewarm);
        assert!(!eco_policy.preview_keep_alive);
        assert_eq!(eco_policy.hover_delay_ms, 350);
    }

    #[test]
    #[cfg(windows)]
    fn test_system_memory_query_runs() {
        let mem = query_system_memory();
        assert!(mem.is_some());
        let info = mem.unwrap();
        assert!(info.total_phys_bytes > 0);
        assert!(info.avail_phys_bytes > 0);
        assert!(info.memory_load_pct <= 100);
    }
}
