//! Presentation messages consumed by the GUI reducer.

use crate::{BridgeEvent, ContrastPreference, MotionPreference, Route};
use denon_avr_application::ReceiverSelection;
use denon_avr_domain::{ConfiguredReceivers, ListeningModeGroup, QuickSelectSlot};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub enum Message {
    ConfigLoaded(Result<ConfiguredReceivers, String>),
    CommandFinished(Result<(), String>),
    Navigate(Route),
    AddressChanged(String),
    NameChanged(String),
    Discover,
    DiscoveryFinished(Result<Vec<denon_avr_domain::DiscoveredReceiver>, String>),
    SaveDiscovered(denon_avr_domain::DiscoveredReceiver),
    DiscoveredSaved(Result<(ConfiguredReceivers, ReceiverSelection), String>),
    ManualSetup,
    ManualSaved(Result<(ConfiguredReceivers, ReceiverSelection), String>),
    Select(ReceiverSelection),
    Connect,
    Refresh,
    Disconnect,
    /// Toggle the receiver's Main Zone (Zone 1) power state.
    ToggleMainZonePower,
    Mute,
    Unmute,
    VolumeChanged(f32),
    CommitVolume,
    AdjustVolume(f32),
    HideVolumeValue(u64),
    LaunchTick,
    SelectListeningModeGroup(ListeningModeGroup),
    SelectSurroundMode(String),
    OpenSourcePicker,
    CloseSourcePicker,
    SelectInput(String),
    RefreshQuickSelectEq,
    RefreshSourceCatalog,
    RecallQuickSelect(QuickSelectSlot),
    Shutdown,
    ToggleMessages,
    ClearMessages,
    SetTextScale(u8),
    SetMotion(MotionPreference),
    SetContrast(ContrastPreference),
    CaptureVisual,
    ScreenshotCaptured(iced::window::Screenshot),
    ScreenshotWritten(Result<PathBuf, String>),
    Keyboard(iced::keyboard::Event),
    Bridge(Box<BridgeEvent>),
}
