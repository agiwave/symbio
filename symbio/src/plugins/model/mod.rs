//! Universal Model Agent Engine
//!
//! Supports: Any OpenAI-compatible endpoint (OpenAI, Azure, LMStudio, etc.)
//!           Future providers: Anthropic Claude, Google Gemini, etc.
//!
//! Module layout:
//! - `types`:           Unified type definitions (ModelConfig, NativeMessage, etc.)
//! - `token`:           Token estimation and context management
//! - `context`:         HTTP client singleton + ChatOrchestrator struct
//! - `chat_loop`:       Main chat loop and turn processing orchestration
//! - `turn_processor`:  Single turn processing (request, response, tools)
//! - `session_context`: Session context load, history reconstruction, tool normalization
//! - `tool_executor`:   Single tool execution, approval flow, batch tool dispatch
//! - `resume`:          Tool call resume (approve/reject/retry/supply/answer) in chat_loop
//! - `message_builder`: NativeMessage construction and session persistence
//! - `approval`:        Tool invocation approval gate
//! - `handlers`:        Non-streaming handlers (status, list_models, config)
//! - `tool_call`:       Streaming tool-call accumulator
//! - `plugin`:          Core ModelPlugin entry point + factory registration

mod chat_loop;
mod compression;
mod context;
mod handlers;
pub mod message_builder;
mod plugin;
mod protocol;
mod protocols;
pub mod resume;
mod tool_call;
pub mod tool_executor;
mod turn_processor;
mod types;
