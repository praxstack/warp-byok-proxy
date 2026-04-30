#![deny(clippy::wildcard_enum_match_arm)]
#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

pub mod auth;
pub mod betas;
pub mod cache;
pub mod cert;
pub mod config;
pub mod frame;
pub mod model_id;
pub mod stream_accumulator;
pub mod thinking;
pub mod ui_adapter;
