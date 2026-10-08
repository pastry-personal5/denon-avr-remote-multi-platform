//! Volume as the rules see it: whole numbers of half decibels.

use denon_avr_domain::MasterVolume;
use std::fmt;

/// The receiver's `Minimum`, which the rules treat as -80.0 dB.
const LOWEST: i16 = -160;
const HIGHEST: i16 = 36;
/// The widest distance on the scale, in half decibels.
const WIDEST_SPAN: u16 = (HIGHEST - LOWEST) as u16;

/// Why a number is not a usable limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LevelError {
    /// Not a multiple of 0.5 dB, or not a number.
    OffGrid,
    /// Outside the receiver's scale.
    OutOfRange,
}

impl fmt::Display for LevelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OffGrid => f.write_str("must be a multiple of 0.5 dB"),
            Self::OutOfRange => f.write_str("is outside the receiver's volume scale"),
        }
    }
}

impl std::error::Error for LevelError {}

/// A point on the volume scale, in half decibels. `Minimum` is -160 (-80.0 dB),
/// half a decibel under the lowest numbered level, -79.5 dB.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Level(i16);

impl Level {
    pub const MINIMUM: Level = Level(LOWEST);

    /// The level of an observed or requested volume.
    pub fn of(volume: MasterVolume) -> Self {
        match volume {
            MasterVolume::Minimum => Self::MINIMUM,
            MasterVolume::DbHalfSteps(steps) => Self(steps),
        }
    }

    /// A level in decibels. It must lie on the 0.5 dB grid between -80.0 and
    /// +18.0 inclusive.
    pub fn from_db(db: f64) -> Result<Self, LevelError> {
        Self::from_half_steps(grid(db)?)
    }

    pub fn from_half_steps(steps: impl Into<i64>) -> Result<Self, LevelError> {
        match i16::try_from(steps.into()) {
            Ok(steps) if (LOWEST..=HIGHEST).contains(&steps) => Ok(Self(steps)),
            _ => Err(LevelError::OutOfRange),
        }
    }

    pub fn half_steps(self) -> i16 {
        self.0
    }

    pub fn db(self) -> f64 {
        f64::from(self.0) / 2.0
    }

    /// How far this level is above `floor`, in half decibels. Negative when it
    /// is below. Wide enough that no pair of levels overflows.
    pub fn above(self, floor: Level) -> i32 {
        i32::from(self.0) - i32::from(floor.0)
    }
}

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.1} dB", self.db())
    }
}

/// A distance on the volume scale, in half decibels: a step or a budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Span(u16);

impl Span {
    /// A distance in decibels, from 0 up to the width of the scale, on the
    /// 0.5 dB grid.
    pub fn from_db(db: f64) -> Result<Self, LevelError> {
        Self::from_half_steps(grid(db)?)
    }

    pub fn from_half_steps(steps: impl Into<i64>) -> Result<Self, LevelError> {
        match u16::try_from(steps.into()) {
            Ok(steps) if steps <= WIDEST_SPAN => Ok(Self(steps)),
            _ => Err(LevelError::OutOfRange),
        }
    }

    pub fn half_steps(self) -> u16 {
        self.0
    }

    pub fn db(self) -> f64 {
        f64::from(self.0) / 2.0
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.1} dB", self.db())
    }
}

/// Decibels as whole half decibels, or the reason they are not.
fn grid(db: f64) -> Result<i64, LevelError> {
    let half = db * 2.0;
    if !half.is_finite() {
        return Err(LevelError::OutOfRange);
    }
    if half.abs() > 1_000.0 {
        return Err(LevelError::OutOfRange);
    }
    if half.fract() != 0.0 {
        return Err(LevelError::OffGrid);
    }
    Ok(half as i64)
}
