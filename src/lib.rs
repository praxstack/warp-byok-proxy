#![deny(clippy::wildcard_enum_match_arm)]
#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

pub mod auth;
pub mod bedrock_client;
pub mod betas;
pub mod cache;
pub mod cert;
pub mod config;
pub mod frame;
pub mod model_id;
pub mod route_multi_agent;
pub mod server;
pub mod stream_accumulator;
pub mod thinking;
pub mod translator;
pub mod ui_adapter;
