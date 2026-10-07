//! Presentation messages consumed by the GUI reducer.

use crate::{BridgeEvent, ContrastPreference, MotionPreference, Route, Selection};
use denon_avr_domain::{ConfiguredReceivers, PowerState, SoundModeCategory};
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
    DiscoveredSaved(Result<(ConfiguredReceivers, Selection), String>),
    ManualSetup,
    ManualSaved(Result<(ConfiguredReceivers, Selection), String>),
    Select(Selection),
    Refresh,
    /// Read the receiver's HTTP information again, on the timer.
    HttpTick,
    /// Toggle the receiver's Main Zone (Zone 1) power state.
    ToggleMainZonePower,
    ToggleZone2Power,
    OpenMainZonePowerPopup,
    OpenZone2PowerPopup,
    ClosePowerPopup,
    SetMainZonePower(PowerState),
    SetZone2Power(PowerState),
    Mute,
    Unmute,
    VolumeChanged(f32),
    CommitVolume,
    AdjustVolume(f32),
    HideVolumeValue(u64),
    LaunchTick,
    SelectSoundModeCategory(SoundModeCategory),
    SelectSurroundMode(SoundModeCategory, String),
    SoundModeControlTimedOut(u64),
    ToggleSoundModeFavorite(String),
    SoundModeFavoritesSaved(Result<ConfiguredReceivers, String>),
    OpenSourcePicker,
    CloseSourcePicker,
    SelectInput(String),
    RefreshSourceCatalog,
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
