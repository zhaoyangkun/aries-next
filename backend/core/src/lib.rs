// async_trait 生成的 `Pin<Box<dyn Future>>` 返回值自带 #[must_use]，clippy 1.99 新增的
// double_must_use 会对所有 trait 方法误报；这些 Future 本来就应当被 await，此处整 crate 豁免。
#![allow(clippy::double_must_use)]

pub mod ai;
pub mod ai_prompts;
pub mod auth;
pub mod chunking;
pub mod comments;
pub mod content;
pub mod galleries;
pub mod health;
pub mod jobs;
pub mod journals;
pub mod links;
pub mod logs;
pub mod media;
pub mod navigation;
pub mod pages;
pub mod retrieval;
pub mod search;
pub mod settings;
