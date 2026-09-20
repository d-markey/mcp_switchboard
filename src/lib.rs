//! Switchboard is an MCP (Model Context Protocol) reverse proxy and aggregator.
//! It allows developers to route LLM requests to various backend services, managing
//! namespaces via tool prefixes, filtering available tools dynamically, and rewriting
//! requests and responses to optimize context window utilization.

pub mod config;
pub mod proxy;
pub mod transform;
pub mod transform_md_tables;
pub mod tool_filter;
pub mod tool_search;
pub mod jsonrpc;
pub mod sse;
pub mod transform_toon;
pub mod backend_logging;
