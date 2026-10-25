//! Native Linux telemetry capture adapters with consent gating and fakeable seams.
//!
//! Enforces:
//! - Phase 2 First Bounded Slice:
//!   - X11 active window detection via EWMH (`_NET_ACTIVE_WINDOW`, `_NET_WM_NAME`, `WM_CLASS`, `_NET_WM_PID`).
//!   - User-space `/proc` process inventory with pure executable name extraction (`file_name`).
//!   - Program list aggregation capped at [`MAX_RUNNING_PROGRAMS`] (200) with case-insensitive deduplication.
//!   - Honest Wayland unsupported behavior (`WaylandActiveWindowProvider`).
//!   - Consent-gated acquisition (`ConsentGatedTracker`) with ticket generation invalidation (ABA protection).
//!   - Side-effect-free fake provider (`FakeAcquisitionProvider`) matching Windows test infrastructure.

pub mod fake;
pub mod gated;
pub mod model;
pub mod procfs;
pub mod traits;
pub mod wayland;
pub mod x11;

pub use fake::FakeAcquisitionProvider;
pub use gated::ConsentGatedTracker;
pub use model::{
    aggregate_running_programs, file_name, is_kernel_thread, program_list, WindowInfo,
    MAX_RUNNING_PROGRAMS,
};
pub use procfs::{
    MockProcFsReader, MockProcessEntry, ProcFsReader, ProcFsRunningAppsProvider, SysProcFsReader,
};
pub use traits::{
    AcquisitionProvider, AcquisitionProviderExt, ActiveWindowProvider, RunningAppsProvider,
    TrackingError,
};
pub use wayland::WaylandActiveWindowProvider;
pub use x11::{
    craft_get_property_wire_reply, MockX11PropertyReader, RawPropertyReply, SyntheticX11Transport,
    SysX11PropertyReader, SysX11Transport, X11ActiveWindowProvider, X11PropertyReader,
    X11PropertyTransport,
};
