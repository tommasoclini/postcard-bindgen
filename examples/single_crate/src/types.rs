//! Wire types shared by the Rust library and the C binding build script.
//!
//! Field order and enum variant order are part of the wire format.

use serde::{Deserialize, Serialize};

/// Angular velocity in radians per second along the sensor's X, Y, and Z axes.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "c-bindings", derive(postcard_bindgen::PostcardBindings))]
pub struct Gyro {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// Acceleration in meters per second squared along the sensor's X, Y, and Z axes.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "c-bindings", derive(postcard_bindgen::PostcardBindings))]
pub struct Acc {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// One inertial measurement with a monotonic device timestamp in microseconds.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "c-bindings", derive(postcard_bindgen::PostcardBindings))]
pub struct ImuSample {
    pub timestamp_us: u64,
    pub gyro: Gyro,
    pub acc: Acc,
    pub temperature_c: Option<f32>,
}

/// Requested measurement frequency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "c-bindings", derive(postcard_bindgen::PostcardBindings))]
pub enum SamplingRate {
    Hz25,
    Hz50,
    Hz100,
    Hz200,
}

/// Selects the measurement frequency and enabled sensors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "c-bindings", derive(postcard_bindgen::PostcardBindings))]
pub struct SensorConfig {
    pub sampling_rate: SamplingRate,
    pub gyro_enabled: bool,
    pub acc_enabled: bool,
}

/// Commands sent to the sensor device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "c-bindings", derive(postcard_bindgen::PostcardBindings))]
pub enum Request {
    GetSample,
    Configure(SensorConfig),
}

/// Replies sent by the sensor device.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "c-bindings", derive(postcard_bindgen::PostcardBindings))]
pub enum Response {
    Sample { sequence: u32, sample: ImuSample },
    Configured,
    Error { code: u16 },
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "c-bindings", derive(postcard_bindgen::PostcardBindings))]
pub struct Log<'a> {
    level: u8,
    s: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "c-bindings", derive(postcard_bindgen::PostcardBindings))]
pub struct Log2<'a> {
    level: u8,
    s: &'a [u8],
}

// TODO: test strings and byte arrays
