#[cfg(feature = "api-clients")]
pub mod openai;

#[cfg(feature = "api-clients")]
pub mod openai_image;

#[cfg(feature = "api-clients")]
pub mod openai_stt;

#[cfg(feature = "realtime-clients")]
pub mod openai_realtime;

pub mod map;
pub mod multi;
pub mod tester;
