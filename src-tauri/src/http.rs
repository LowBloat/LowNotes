//! Long-lived HTTP clients. Building a reqwest client per request throws away
//! its connection pool, DNS cache and TLS sessions.

use std::{sync::OnceLock, time::Duration};

use reqwest::{Client, ClientBuilder};

/// General-purpose client; set a per-request `.timeout()` for each call.
pub fn client() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    cached(&CLIENT, |builder| builder)
}

/// A client with its own configuration, built once into `slot`.
pub fn cached(
    slot: &'static OnceLock<Client>,
    configure: impl FnOnce(ClientBuilder) -> ClientBuilder,
) -> &'static Client {
    slot.get_or_init(|| {
        configure(Client::builder().connect_timeout(Duration::from_secs(15)))
            .build()
            // Only fails when the TLS backend cannot start; `Client::new` reports that.
            .unwrap_or_else(|_| Client::new())
    })
}
