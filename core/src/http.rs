//! One shared HTTP client. rustls with the `ring` provider and bundled Mozilla roots, so the same
//! code works on desktop and Android without JNI setup for the platform verifier.

use std::sync::{Arc, LazyLock};
use std::time::Duration;

pub static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    let roots = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
    let tls = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .expect("tls versions")
        .with_root_certificates(roots)
        .with_no_client_auth();
    reqwest::Client::builder()
        .tls_backend_preconfigured(tls)
        .timeout(Duration::from_secs(15))
        .build()
        .expect("http client")
});
