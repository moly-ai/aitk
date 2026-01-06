#[cfg(feature = "api-clients")]
pub mod openai;

#[cfg(feature = "api-clients")]
pub mod openai_image;

#[cfg(feature = "realtime-clients")]
pub mod openai_realtime;

pub mod map;
pub mod multi;
pub mod tester;
