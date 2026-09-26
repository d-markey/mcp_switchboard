//! Switchboard is an MCP (Model Context Protocol) reverse proxy and aggregator.
//! It allows developers to route LLM requests to various backend services, managing
//! namespaces via tool prefixes, filtering available tools dynamically, and rewriting
//! requests and responses to optimize context window utilization.

pub mod config;
pub mod proxy;
pub mod transform;
pub mod transform_md_tables;
pub mod transform_csv;
pub mod tool_filter;
pub mod tool_search;
pub mod jsonrpc;
pub mod sse;
pub mod transform_toon;
pub mod backend_logging;
pub mod http_utils;
pub mod health;
pub mod backend_stats;
pub mod tool_utils;
