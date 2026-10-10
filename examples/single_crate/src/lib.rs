//! A shared, sensor protocol.
//!
//! The `c-bindings` feature generates C bindings from these same types during a
//! native host build. Leave it disabled when using this crate in firmware.

#![no_std]

pub mod types;
pub use types::{Acc, Gyro, ImuSample, Request, Response, SamplingRate, SensorConfig};

/// Directory containing the generated header, source, and postcard-c runtime.
/// Available only in host builds with `c-bindings` enabled.
#[cfg(feature = "c-bindings")]
pub const C_BINDINGS_DIR: &str = env!("SINGLE_CRATE_C_BINDINGS_DIR");
