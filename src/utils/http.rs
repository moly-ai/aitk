#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn default_client() -> reqwest::Client {
    use std::time::Duration;

    // On native, there are no default timeouts. Connection may hang if we don't
    // configure them.
    reqwest::Client::builder()
        // Only considered while establishing the connection.
        .connect_timeout(Duration::from_secs(90))
        // Considered while reading the response and reset on every chunk
        // received.
        //
        // Warning: Do not use normal `timeout` method as it doesn't consider
        // this.
        .read_timeout(Duration::from_secs(90))
        .build()
        .unwrap()
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn default_client() -> reqwest::Client {
    // On web, reqwest timeouts are not configurable, but it uses the browser's
    // fetch API under the hood, which handles connection issues properly.
    reqwest::Client::new()
}
