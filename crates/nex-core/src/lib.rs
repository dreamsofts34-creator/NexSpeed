//! Nex Speed engine: platform-independent sampling and formatting.
//! No OS-specific code lives here, so everything is testable on any machine.

mod connections;
mod export;
mod format;
mod history;
mod prefs;
mod sampler;
mod source;
mod usage;

pub use format::{format_bytes, format_speed, UnitSystem};
pub use history::{Ema, SpeedHistory, HISTORY_LEN};
pub use sampler::{AdapterSpeed, Sampler, Snapshot};
pub use source::{AdapterReading, NetSource};
pub use usage::{limit_percent, DayUsage, UsageLog};
pub use connections::{parse_proc_net_tcp, windows_port, Connection, ProcTcpRow, TcpState};
pub use export::usage_csv;
pub use prefs::{last_days, limit_level, next_opacity, next_style, prev_day, style_name, OPACITY_STEPS};
