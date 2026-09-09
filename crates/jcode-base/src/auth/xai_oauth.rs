//! Native xAI device OAuth. No Grok CLI or external credential imports.
use anyhow::{Context, Result, bail};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

pub const ID: &str = "xai-oauth";
pub const API_BASE: &str = "https://api.x.ai/v1";
const ISSUER: &str = "https://auth.x.ai";
const CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";
const SCOPES: &str = "openid profile email offline_access grok-cli:access api:access";

#[derive(Clone, Serialize, Deserialize)]
pub struct Credentials {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
    #[serde(default)]
    pub blocked: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct DeviceAuthorization {
    device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    pub expires_in: u64,
    #[serde(default = "poll_interval")]
    pub interval: u64,
    #[serde(default)]
    pub expires_at: i64,
}

#[derive(Deserialize)]
struct Tokens {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: u64,
    #[serde(default)]
    token_type: Option<String>,
}

fn poll_interval() -> u64 {
    5
}
fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

pub fn credentials_path() -> Result<PathBuf> {
    Ok(crate::storage::jcode_dir()?.join("xai-oauth.json"))
}
fn pending_path() -> Result<PathBuf> {
    Ok(crate::storage::jcode_dir()?.join("xai-oauth-pending.json"))
}

pub fn client() -> Result<Client> {
    Ok(Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(30))
        .user_agent(concat!("jcode/", env!("CARGO_PKG_VERSION")))
        .build()?)
}

fn atomic_save(path: &Path, value: &impl Serialize) -> Result<()> {
    let dir = path.parent().context("xAI credential path has no parent")?;
    std::fs::create_dir_all(dir)?;
    let mut temp = tempfile::NamedTempFile::new_in(dir)?;
    crate::platform::set_permissions_owner_only(temp.path())?;
    temp.write_all(&serde_json::to_vec_pretty(value)?)?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

fn load_at(path: &Path) -> Result<Credentials> {
    let bytes = std::fs::read(path)
        .context("xAI OAuth credentials not found. Run `jcode login --provider xai-oauth`.")?;
    let credentials: Credentials = serde_json::from_slice(&bytes)
        .context("Invalid xAI OAuth credential file. Run `jcode login --provider xai-oauth`.")?;
    if credentials.access_token.trim().is_empty() || credentials.refresh_token.trim().is_empty() {
        bail!("Incomplete xAI OAuth credentials. Run `jcode login --provider xai-oauth`.");
    }
    Ok(credentials)
}

pub fn load() -> Result<Credentials> {
    load_at(&credentials_path()?)
}
pub fn has_credentials() -> bool {
    load().is_ok_and(|c| c.blocked.is_none())
}

fn validate_authorization(mut auth: DeviceAuthorization) -> Result<DeviceAuthorization> {
    if auth.device_code.is_empty()
        || auth.user_code.is_empty()
        || auth.expires_in == 0
        || auth.expires_in > 86400
    {
        bail!("xAI returned an invalid device grant. Retry `jcode login --provider xai-oauth`.");
    }
    for raw in std::iter::once(&auth.verification_uri).chain(auth.verification_uri_complete.iter())
    {
        let url = reqwest::Url::parse(raw).context("Invalid xAI verification URL")?;
        if url.scheme() != "https"
            || !matches!(url.host_str(), Some("accounts.x.ai" | "auth.x.ai"))
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port().is_some()
        {
            bail!("xAI returned an untrusted verification URL. Do not open it. Retry login later.");
        }
    }
    auth.interval = auth.interval.max(1);
    auth.expires_at = now()
        .checked_add(auth.expires_in as i64)
        .context("Invalid xAI grant expiry")?;
    Ok(auth)
}

pub async fn begin_login() -> Result<DeviceAuthorization> {
    let response = client()?
        .post(format!("{ISSUER}/oauth2/device/code"))
        .form(&[("client_id", CLIENT_ID), ("scope", SCOPES)])
        .send()
        .await?;
    let status = response.status();
    if !status.is_success() {
        bail!(
            "xAI device login returned HTTP {status}. Retry login after checking xAI availability."
        );
    }
    let authorization = validate_authorization(
        response
            .json()
            .await
            .context("Invalid xAI device response")?,
    )?;
    atomic_save(&pending_path()?, &authorization)?;
    Ok(authorization)
}

pub fn pending_login() -> Result<DeviceAuthorization> {
    let auth: DeviceAuthorization = serde_json::from_slice(&std::fs::read(pending_path()?)?)?;
    if auth.expires_at <= now() {
        bail!(
            "xAI device grant expired. Run `jcode login --provider xai-oauth --print-auth-url` again."
        );
    }
    Ok(auth)
}

fn make_credentials(tokens: Tokens, previous_refresh: Option<&str>) -> Result<Credentials> {
    if tokens.access_token.trim().is_empty()
        || tokens.expires_in == 0
        || tokens.expires_in > 366 * 86400
        || tokens
            .token_type
            .as_deref()
            .is_some_and(|s| !s.eq_ignore_ascii_case("bearer"))
    {
        bail!(
            "xAI returned invalid OAuth tokens. Existing credentials were preserved. Retry login."
        );
    }
    let refresh = tokens
        .refresh_token
        .filter(|s| !s.trim().is_empty())
        .or_else(|| previous_refresh.map(str::to_owned))
        .context("xAI did not return a refresh token. Retry login.")?;
    Ok(Credentials {
        access_token: tokens.access_token,
        refresh_token: refresh,
        expires_at: now() + tokens.expires_in as i64,
        blocked: None,
    })
}

fn oauth_failure(status: StatusCode, error: &str) -> anyhow::Error {
    if status == StatusCode::FORBIDDEN {
        anyhow::anyhow!(
            "xAI OAuth access denied (HTTP 403). Check this account's OAuth API entitlement with xAI. Repeated login will not resolve this denial."
        )
    } else if matches!(error, "invalid_grant" | "access_denied" | "expired_token") {
        anyhow::anyhow!(
            "xAI OAuth grant is expired, denied, or revoked. Run `jcode login --provider xai-oauth` when ready to approve access."
        )
    } else {
        anyhow::anyhow!(
            "xAI OAuth request failed (HTTP {status}). Credentials were preserved. Retry after checking xAI availability."
        )
    }
}

pub async fn complete_login(auth: &DeviceAuthorization) -> Result<()> {
    let client = client()?;
    let mut interval = auth.interval.max(1);
    loop {
        let remaining = auth.expires_at.saturating_sub(now());
        if remaining <= interval as i64 {
            bail!("xAI device grant expired. Start a fresh `jcode login --provider xai-oauth`.");
        }
        tokio::time::sleep(Duration::from_secs(interval)).await;
        let response = client
            .post(format!("{ISSUER}/oauth2/token"))
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("client_id", CLIENT_ID),
                ("device_code", auth.device_code.as_str()),
            ])
            .send()
            .await?;
        let status = response.status();
        if status.is_success() {
            let credentials = make_credentials(
                response
                    .json()
                    .await
                    .context("Invalid xAI token response")?,
                None,
            )?;
            let path = credentials_path()?;
            let _lock = credential_lock(&path).await?;
            atomic_save(&path, &credentials)?;
            // Remove only the grant this call completed, not a newer login's state.
            if pending_login().is_ok_and(|pending| pending.device_code == auth.device_code) {
                let _ = std::fs::remove_file(pending_path()?);
            }
            return Ok(());
        }
        let error: serde_json::Value = response.json().await.unwrap_or_default();
        match error["error"].as_str().unwrap_or("") {
            "authorization_pending" => {}
            "slow_down" => interval = interval.saturating_add(5),
            code => return Err(oauth_failure(status, code)),
        }
    }
}

async fn credential_lock(path: &Path) -> Result<File> {
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path.with_extension("lock"))?;
    crate::platform::set_permissions_owner_only(&path.with_extension("lock"))?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(40);
    loop {
        match lock.try_lock() {
            Ok(()) => return Ok(lock),
            Err(std::fs::TryLockError::WouldBlock) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(error) => bail!(
                "xAI credential lock unavailable: {error}. Retry after the active login or refresh completes."
            ),
        }
    }
}

/// `rejected` is the access token that received 401, not an instruction to rotate
/// a different token another process has already refreshed.
pub async fn access_token(rejected: Option<&str>) -> Result<String> {
    token_at(
        &credentials_path()?,
        &format!("{ISSUER}/oauth2/token"),
        rejected,
    )
    .await
}

async fn token_at(path: &Path, endpoint: &str, rejected: Option<&str>) -> Result<String> {
    let _lock = credential_lock(path).await?;
    let mut credentials = load_at(path)?;
    if let Some(reason) = credentials.blocked.as_deref() {
        return Err(oauth_failure(
            if reason == "entitlement" {
                StatusCode::FORBIDDEN
            } else {
                StatusCode::BAD_REQUEST
            },
            "invalid_grant",
        ));
    }
    if credentials.expires_at > now() + 60 && rejected != Some(credentials.access_token.as_str()) {
        return Ok(credentials.access_token);
    }
    let response = client()?
        .post(endpoint)
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", CLIENT_ID),
            ("refresh_token", credentials.refresh_token.as_str()),
        ])
        .send()
        .await?;
    let status = response.status();
    if !status.is_success() {
        let body: serde_json::Value = response.json().await.unwrap_or_default();
        let error = body["error"].as_str().unwrap_or("");
        if status == StatusCode::FORBIDDEN || matches!(error, "invalid_grant" | "access_denied") {
            credentials.blocked = Some(
                if status == StatusCode::FORBIDDEN {
                    "entitlement"
                } else {
                    "reauth"
                }
                .into(),
            );
            atomic_save(path, &credentials)?;
        }
        return Err(oauth_failure(status, error));
    }
    let refreshed = make_credentials(
        response
            .json()
            .await
            .context("Invalid xAI refresh response")?,
        Some(&credentials.refresh_token),
    )?;
    atomic_save(path, &refreshed)?;
    Ok(refreshed.access_token)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn server(status: &str, body: &str) -> (String, std::thread::JoinHandle<String>) {
        use std::io::Read;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let task = std::thread::spawn(move || {
            let started = std::time::Instant::now();
            let mut socket = loop {
                if let Ok((socket, _)) = listener.accept() {
                    break socket;
                }
                assert!(
                    started.elapsed() < Duration::from_secs(5),
                    "mock token request never arrived"
                );
                std::thread::sleep(Duration::from_millis(10));
            };
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut input = Vec::new();
            loop {
                let mut buffer = [0; 4096];
                let count = socket.read(&mut buffer).unwrap();
                if count == 0 {
                    break;
                }
                input.extend_from_slice(&buffer[..count]);
                if let Some(end) = input.windows(4).position(|p| p == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&input[..end]).to_ascii_lowercase();
                    let size: usize = headers
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:"))
                        .unwrap()
                        .trim()
                        .parse()
                        .unwrap();
                    if input.len() >= end + 4 + size {
                        break;
                    }
                }
            }
            socket.write_all(response.as_bytes()).unwrap();
            String::from_utf8(input).unwrap()
        });
        (format!("http://{address}/token"), task)
    }

    #[tokio::test]
    async fn expired_token_refreshes_once_for_concurrent_callers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let mut creds = make_credentials(tokens(), None).unwrap();
        creds.expires_at = 0;
        atomic_save(&path, &creds).unwrap();
        let (url, server) = server(
            "200 OK",
            r#"{"access_token":"new-access","refresh_token":"new-refresh","expires_in":3600}"#,
        );
        let (first, second) =
            tokio::join!(token_at(&path, &url, None), token_at(&path, &url, None));
        assert_eq!(first.unwrap(), "new-access");
        assert_eq!(second.unwrap(), "new-access");
        let request = server.join().unwrap();
        assert!(request.ends_with("refresh_token=refresh"));
        assert!(request.contains("grant_type=refresh_token"));
        assert_eq!(load_at(&path).unwrap().refresh_token, "new-refresh");
    }

    #[tokio::test]
    async fn rejection_refresh_preserves_a_nonrotated_refresh_token() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        atomic_save(&path, &make_credentials(tokens(), None).unwrap()).unwrap();
        let (url, server) = server(
            "200 OK",
            r#"{"access_token":"replacement","expires_in":3600}"#,
        );
        assert_eq!(
            token_at(&path, &url, Some("access")).await.unwrap(),
            "replacement"
        );
        server.join().unwrap();
        assert_eq!(load_at(&path).unwrap().refresh_token, "refresh");
    }

    #[tokio::test]
    async fn revoked_grant_preserves_tokens_and_stops_refresh_retries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let mut creds = make_credentials(tokens(), None).unwrap();
        creds.expires_at = 0;
        atomic_save(&path, &creds).unwrap();
        let (url, server) = server("400 Bad Request", r#"{"error":"invalid_grant"}"#);
        assert!(
            token_at(&path, &url, None)
                .await
                .unwrap_err()
                .to_string()
                .contains("Run `jcode login")
        );
        server.join().unwrap();
        let saved = load_at(&path).unwrap();
        assert_eq!(saved.refresh_token, "refresh");
        assert_eq!(saved.blocked.as_deref(), Some("reauth"));
        assert!(
            token_at(&path, "http://127.0.0.1:1", None)
                .await
                .unwrap_err()
                .to_string()
                .contains("Run `jcode login")
        );
    }

    #[tokio::test]
    async fn transient_refresh_failure_preserves_exact_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let mut creds = make_credentials(tokens(), None).unwrap();
        creds.expires_at = 0;
        atomic_save(&path, &creds).unwrap();
        let before = std::fs::read(&path).unwrap();
        let (url, server) = server(
            "503 Service Unavailable",
            r#"{"error":"temporarily_unavailable"}"#,
        );
        assert!(token_at(&path, &url, None).await.is_err());
        server.join().unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
    fn tokens() -> Tokens {
        Tokens {
            access_token: "access".into(),
            refresh_token: Some("refresh".into()),
            expires_in: 3600,
            token_type: Some("Bearer".into()),
        }
    }
    #[test]
    fn validates_tokens_and_preserves_unrotated_refresh() {
        let mut t = tokens();
        t.refresh_token = None;
        assert!(make_credentials(tokens(), None).is_ok());
        assert_eq!(
            make_credentials(t, Some("previous")).unwrap().refresh_token,
            "previous"
        );
        let mut t = tokens();
        t.access_token.clear();
        assert!(make_credentials(t, None).is_err());
        let mut t = tokens();
        t.expires_in = 0;
        assert!(make_credentials(t, None).is_err());
        let mut t = tokens();
        t.token_type = Some("MAC".into());
        assert!(make_credentials(t, None).is_err());
    }
    #[test]
    fn atomic_credentials_are_private_and_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let creds = make_credentials(tokens(), None).unwrap();
        atomic_save(&path, &creds).unwrap();
        assert_eq!(load_at(&path).unwrap().access_token, "access");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
    #[tokio::test]
    async fn fresh_credentials_do_not_call_token_endpoint() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        atomic_save(&path, &make_credentials(tokens(), None).unwrap()).unwrap();
        assert_eq!(
            token_at(&path, "http://127.0.0.1:1", None).await.unwrap(),
            "access"
        );
        assert_eq!(
            token_at(&path, "http://127.0.0.1:1", Some("older-token"))
                .await
                .unwrap(),
            "access"
        );
    }
    #[test]
    fn entitlement_failure_does_not_prescribe_relogin() {
        let text = oauth_failure(StatusCode::FORBIDDEN, "access_denied").to_string();
        assert!(text.contains("entitlement"));
        assert!(!text.contains("Run `jcode login"));
    }
    #[test]
    fn verification_urls_are_first_party_only() {
        let make = |url: &str| DeviceAuthorization {
            device_code: "secret".into(),
            user_code: "CODE".into(),
            verification_uri: url.into(),
            verification_uri_complete: None,
            expires_in: 1800,
            interval: 5,
            expires_at: 0,
        };
        assert!(validate_authorization(make("https://accounts.x.ai/oauth2/device")).is_ok());
        for url in [
            "http://accounts.x.ai/oauth2/device",
            "https://accounts.x.ai.evil.test/",
            "https://user@accounts.x.ai/",
        ] {
            assert!(validate_authorization(make(url)).is_err());
        }
    }
}
