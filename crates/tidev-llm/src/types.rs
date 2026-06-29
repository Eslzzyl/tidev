//! LLM-provider-agnostic config types for the LLM layer.
//!
//! The [`LlmProviderConfig`] type has moved to `tidev-types` to break the
//! `tidev-config` → `tidev-llm` dependency: config loads `tidev-types`
//! instead of the LLM implementation crate.
//!
//! This module re-exports it for backward compatibility — all provider
//! implementations can continue to use `crate::types::LlmProviderConfig`.

pub use tidev_types::LlmProviderConfig;
