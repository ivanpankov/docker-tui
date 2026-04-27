//! Docker supervision TUI: Ratatui UI, Bollard-backed Docker access, async event runtime.

pub mod app;
pub mod docker;
pub mod events;
pub mod runtime;
pub mod ui;

pub use app::run;
