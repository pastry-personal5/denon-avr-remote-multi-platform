//! The window's private state, grouped by what it serves: the volume control,
//! the sound mode controls, the supplemental reads, capture, and launch.

use denon_avr_domain::{ConfiguredReceivers, SoundModeCategory};
use std::path::PathBuf;

/// The volume control: the transient value display and the volume command the
/// receiver has yet to confirm.
pub(super) struct VolumeControl {
    pub(super) value: Option<f32>,
    pub(super) value_request_id: u64,
    pub(super) command_pending: bool,
    /// True after the receiver has reported a volume or the user has
    /// initialized an unknown volume once from the safe minimum.
    pub(super) baseline_initialized: bool,
    /// Request identity is separate from the GUI-wide freshness counter:
    /// background catalog/status reads may legitimately advance that counter
    /// while a volume confirmation is still in flight.
    pub(super) command_request_id: Option<u64>,
}

impl VolumeControl {
    /// Forget the volume command in flight.
    pub(super) fn clear_command(&mut self) {
        self.command_pending = false;
        self.command_request_id = None;
    }
}

/// The sound mode controls: the category filter, the sound mode request in
/// flight, and the serialized saving of the sound mode favorites.
pub(super) struct SoundModeUi {
    /// Local table filter only. The AVR reports a detailed MS mode, not a
    /// category, so this value never represents receiver-observed state.
    pub(super) category_filter: Option<SoundModeCategory>,
    pub(super) request_id: Option<u64>,
    pub(super) save_in_flight: bool,
    pub(super) pending_config: Option<ConfiguredReceivers>,
}

impl SoundModeUi {
    /// Forget the category filter and the sound mode request in flight.
    pub(super) fn clear_request(&mut self) {
        self.category_filter = None;
        self.request_id = None;
    }
}

/// The supplemental reads in flight: the HTTP information, the source catalog,
/// and the Quick Select names.
pub(super) struct SupplementalReads {
    pub(super) http_read_in_flight: bool,
    pub(super) http_read_again: bool,
    pub(super) catalog_read_in_flight: bool,
    pub(super) names_read_in_flight: bool,
    /// The connection generation the source catalog was last requested for on
    /// its own, so a receiver that cannot answer is asked once per connection.
    pub(super) catalog_auto_generation: Option<u64>,
}

/// Where a visual capture is written and which capture scenario is shown.
pub(super) struct CaptureSettings {
    pub(super) directory: Option<PathBuf>,
    pub(super) scenario: Option<String>,
}

/// The launch and wait animation: its frame, and how long the window has
/// waited for the receiver's core status.
pub(super) struct LaunchProgress {
    pub(super) frame: u8,
    pub(super) status_wait_ticks: u16,
}
