cfg_if::cfg_if! {
    if #[cfg(feature = "http")] {
        pub mod openai;
        pub mod openai_image;
        pub mod openai_realtime;
    }
}

pub mod map;
pub mod multi;
pub mod tester;
