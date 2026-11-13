//! Admin API 模块
//!
//! 提供凭据管理和监控功能的 HTTP API
//!
//! # 功能
//! - 查询所有凭据状态
//! - 启用/禁用凭据
//! - 修改凭据优先级
//! - 重置失败计数
//! - 查询凭据余额
//!
//! # 使用
//! ```ignore
//! let admin_service = AdminService::new(token_manager.clone(), endpoint_names);
//! let admin_state = AdminState::new(admin_api_key, admin_service, account_state);
//! let admin_router = create_admin_router(admin_state);
//! ```

mod monitoring;
mod tracker;
pub mod usage_types;
pub use tracker::{RequestTracker, TrackedRequestGuard};
pub mod core_config;
mod error;
mod handlers;
mod middleware;
mod router;
mod service;
pub mod types;

pub use middleware::{AdminState, admin_auth_middleware};
pub use router::create_admin_router;
pub use service::AdminService;
