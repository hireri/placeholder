//! Spotify through librespot: one approval in the user's own browser, then short-lived web-player tokens minted from the
//! saved login. No webview and no developer app.

use librespot_core::{authentication::Credentials, config::SessionConfig, error::ErrorKind, session::Session};
use librespot_protocol::authentication::AuthenticationType;
use std::time::Duration;

/// Spotify's own desktop client identity, the one librespot streams with.
const CLIENT_ID: &str = "65b708073fc0480ea92a077233ca87bd";
const REDIRECT_ADDR: &str = "127.0.0.1:8898";
/// Exit codes the app and the daemon tell apart from a plain failure (1).
const REJECTED: i32 = 2;
const PORT_BUSY: i32 = 3;

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Login {
    /// Kept so Spotify sees the same device every time.
    pub device_id: String,
    pub credentials: Credentials,
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio runtime")
}

/// Opens the browser, waits for approval and hands the reusable login to the daemon.
pub fn sign_in() -> ! {
    if std::net::TcpListener::bind(REDIRECT_ADDR).is_err() {
        eprintln!("port 8898 is in use");
        std::process::exit(PORT_BUSY);
    }
    // A closed browser tab never calls back.
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(300));
        std::process::exit(1);
    });
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let token = librespot_oauth::OAuthClientBuilder::new(CLIENT_ID, &format!("http://{REDIRECT_ADDR}/login"), vec!["streaming"])
            .open_in_browser()
            .with_custom_message("<html><body>Signed in. You can close this tab and go back to Serafin.</body></html>")
            .build()?
            .get_access_token()?;
        let login = runtime().block_on(async {
            let config = SessionConfig::default();
            let device_id = config.device_id.clone();
            let session = Session::new(config, None);
            session.connect(Credentials::with_access_token(token.access_token), false).await?;
            let credentials = Credentials {
                username: Some(session.username()),
                auth_type: AuthenticationType::AUTHENTICATION_STORED_SPOTIFY_CREDENTIALS,
                auth_data: session.auth_data(),
            };
            Ok::<_, librespot_core::Error>(Login { device_id, credentials })
        })?;
        super::post("/auth/spotify", serde_json::json!({ "login": login }))?;
        // The cookie profile left by the old webview sign-in is of no further use.
        let _ = std::fs::remove_dir_all(super::data_dir("spotify"));
        Ok(())
    })();
    match result {
        Ok(()) => std::process::exit(0),
        Err(e) => {
            eprintln!("spotify sign-in: {e}");
            std::process::exit(1)
        }
    }
}

/// For the daemon: reads the saved login from `SPOTIFY_LOGIN`, prints `{bearer, clientToken, expiresIn}`.
pub fn token() -> ! {
    let Some(login) = std::env::var("SPOTIFY_LOGIN").ok().and_then(|v| serde_json::from_str::<Login>(&v).ok()) else {
        std::process::exit(REJECTED);
    };
    let minted = runtime().block_on(async {
        tokio::time::timeout(Duration::from_secs(20), async {
            let session = Session::new(SessionConfig { device_id: login.device_id, ..SessionConfig::default() }, None);
            session.connect(login.credentials, false).await?;
            let token = session.login5().auth_token().await?;
            let client_token = session.spclient().client_token().await?;
            Ok::<_, librespot_core::Error>(serde_json::json!({
                "bearer": token.access_token, "clientToken": client_token, "expiresIn": token.expires_in.as_secs(),
            }))
        })
        .await
    });
    match minted {
        Ok(Ok(json)) => {
            println!("{json}");
            std::process::exit(0)
        }
        Ok(Err(e)) => {
            eprintln!("spotify token: {e}");
            // Only a refused login asks the user to sign in again; anything else is retried later.
            std::process::exit(if e.kind == ErrorKind::PermissionDenied { REJECTED } else { 1 })
        }
        Err(_) => {
            eprintln!("spotify token: timed out");
            std::process::exit(1)
        }
    }
}
