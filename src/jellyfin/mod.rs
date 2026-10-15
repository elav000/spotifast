//! Jellyfin's own catalog and Quick Connect, independent of Spotify grants.
//!
//! Call on the backend runtime. Tokens and temporary Quick Connect secrets
//! deliberately have no Debug/Serialize implementation and never enter URLs
//! except the server-required Quick Connect polling query. Errors omit URLs.

use std::ops::Range;
use std::time::Duration;

use reqwest::{Client as HttpClient, Method, Request, Url};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::settings::ProxyConfig;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("Enter an HTTP or HTTPS Jellyfin server without credentials, query or fragment.")]
    InvalidServer,
    #[error("Invalid Jellyfin identifier or credential.")]
    InvalidValue,
    #[error("Could not configure the Jellyfin connection.")]
    Configuration,
    #[error("Could not reach the Jellyfin server.")]
    Network,
    #[error("Jellyfin returned HTTP {0}.")]
    Http(u16),
    #[error("Jellyfin returned an invalid response.")]
    InvalidResponse,
    #[error("Quick Connect is waiting for approval.")]
    Pending,
}

/// Opaque credential. Persist only through a protected credential store.
pub struct Token(pub(crate) String);

impl Token {
    pub fn new(value: String) -> Result<Self, Error> {
        validate_header_value(&value)?;
        Ok(Self(value))
    }
}

/// Display the code; keep the secret in memory only until completion/cancel.
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Challenge {
    pub code: String,
    secret: String,
}

/// A successful approval. The caller must store the token before saving the
/// non-secret account configuration. This type must never be logged.
pub struct Account {
    pub user_id: String,
    pub server_id: String,
    pub token: Token,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub struct ServerInfo {
    pub id: String,
    pub server_name: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub struct MusicLibrary {
    pub id: String,
    pub name: String,
    pub collection_type: Option<String>,
}

/// Jellyfin IDs stay Jellyfin IDs, never synthetic Spotify URIs.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub struct Item {
    pub id: String,
    pub name: Option<String>,
    pub album: Option<String>,
    pub album_id: Option<String>,
    #[serde(default)]
    pub artists: Vec<String>,
    pub run_time_ticks: Option<u64>,
    pub index_number: Option<u32>,
    pub parent_index_number: Option<u32>,
}

impl Item {
    pub fn duration_ms(&self) -> Option<u64> {
        self.run_time_ticks.map(|ticks| ticks / 10_000)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub struct Page<T> {
    pub items: Vec<T>,
    pub total_record_count: u32,
    pub start_index: u32,
}

pub struct Client {
    http: HttpClient,
    base: Url,
    authorization: String,
}

impl Client {
    /// Redirects are disabled, including same-origin redirects: a configured
    /// base address must not forward an access token to another endpoint.
    /// Build a new client after a proxy change.
    pub fn new(server: &str, device_id: &str, proxy: &ProxyConfig) -> Result<Self, Error> {
        validate_header_value(device_id)?;
        let base = normalize_server(server)?;
        let http = crate::http::build_collection_client(proxy).map_err(|_| Error::Configuration)?;
        let authorization = format!(
            "MediaBrowser Client=\"Spotifast\", Device=\"Desktop\", DeviceId=\"{device_id}\", Version=\"{}\"",
            env!("CARGO_PKG_VERSION")
        );
        Ok(Self {
            http,
            base,
            authorization,
        })
    }

    pub fn server_url(&self) -> &str {
        self.base.as_str()
    }

    pub async fn server_info(&self) -> Result<ServerInfo, Error> {
        self.json(self.request(Method::GET, &["System", "Info", "Public"], None)?)
            .await
    }

    pub async fn initiate(&self) -> Result<Challenge, Error> {
        let request = self.request(Method::POST, &["QuickConnect", "Initiate"], None)?;
        let challenge: Challenge = self
            .json(with_json(request, &serde_json::json!({}))?)
            .await?;
        if challenge.code.is_empty() || challenge.secret.is_empty() {
            return Err(Error::InvalidResponse);
        }
        Ok(challenge)
    }

    pub async fn approved(&self, challenge: &Challenge) -> Result<bool, Error> {
        #[derive(Deserialize)]
        struct State {
            #[serde(rename = "Authenticated")]
            authenticated: bool,
        }
        let mut request = self.request(Method::GET, &["QuickConnect", "Connect"], None)?;
        request
            .url_mut()
            .query_pairs_mut()
            .append_pair("Secret", &challenge.secret);
        let state: State = self.json(request).await?;
        Ok(state.authenticated)
    }

    /// Polling cadence/cancellation belongs to the backend, not this client.
    pub async fn finish(&self, challenge: &Challenge) -> Result<Account, Error> {
        if !self.approved(challenge).await? {
            return Err(Error::Pending);
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct Response {
            access_token: String,
            server_id: String,
            user: User,
        }
        #[derive(Deserialize)]
        struct User {
            #[serde(rename = "Id")]
            id: String,
        }
        let request = self.request(
            Method::POST,
            &["Users", "AuthenticateWithQuickConnect"],
            None,
        )?;
        let response: Response = self
            .json(with_json(
                request,
                &serde_json::json!({"Secret": challenge.secret}),
            )?)
            .await?;
        validate_id(&response.user.id)?;
        validate_id(&response.server_id)?;
        Ok(Account {
            user_id: response.user.id,
            server_id: response.server_id,
            token: Token::new(response.access_token)?,
        })
    }

    pub async fn music_libraries(
        &self,
        user: &str,
        token: &Token,
    ) -> Result<Vec<MusicLibrary>, Error> {
        validate_id(user)?;
        let mut request = self.request(Method::GET, &["UserViews"], Some(token))?;
        request
            .url_mut()
            .query_pairs_mut()
            .append_pair("UserId", user);
        let page: Page<MusicLibrary> = self.json(request).await?;
        Ok(page
            .items
            .into_iter()
            .filter(|view| {
                view.collection_type
                    .as_deref()
                    .is_some_and(|kind| kind.eq_ignore_ascii_case("music"))
            })
            .collect())
    }

    pub async fn albums(
        &self,
        user: &str,
        library: &str,
        start: u32,
        token: &Token,
    ) -> Result<Page<Item>, Error> {
        self.items(user, library, "MusicAlbum", "SortName", start, token)
            .await
    }

    pub async fn album_tracks(
        &self,
        user: &str,
        album: &str,
        start: u32,
        token: &Token,
    ) -> Result<Page<Item>, Error> {
        self.items(
            user,
            album,
            "Audio",
            "ParentIndexNumber,IndexNumber,SortName",
            start,
            token,
        )
        .await
    }

    async fn items(
        &self,
        user: &str,
        parent: &str,
        kind: &str,
        sort: &str,
        start: u32,
        token: &Token,
    ) -> Result<Page<Item>, Error> {
        validate_id(user)?;
        validate_id(parent)?;
        let mut request = self.request(Method::GET, &["Items"], Some(token))?;
        request
            .url_mut()
            .query_pairs_mut()
            .append_pair("UserId", user)
            .append_pair("ParentId", parent)
            .append_pair("IncludeItemTypes", kind)
            .append_pair("Recursive", "true")
            .append_pair("Fields", "Artists,AlbumId")
            .append_pair("SortBy", sort)
            .append_pair("SortOrder", "Ascending")
            .append_pair("StartIndex", &start.to_string())
            .append_pair("Limit", "100")
            .append_pair("EnableTotalRecordCount", "true");
        let page: Page<Item> = self.json(request).await?;
        if page.start_index != start || page.items.len() > 100 {
            return Err(Error::InvalidResponse);
        }
        Ok(page)
    }

    /// Builds an authenticated original-audio request. This is not a decoder
    /// or a promise that byte ranges alone provide accurate time-based seeks.
    pub fn stream_request(
        &self,
        item: &str,
        token: &Token,
        range: Option<Range<u64>>,
    ) -> Result<Request, Error> {
        validate_id(item)?;
        let mut request = self.request(Method::GET, &["Audio", item, "stream"], Some(token))?;
        request
            .url_mut()
            .query_pairs_mut()
            .append_pair("static", "true");
        if let Some(range) = range {
            if range.start >= range.end {
                return Err(Error::InvalidValue);
            }
            request.headers_mut().insert(
                reqwest::header::RANGE,
                format!("bytes={}-{}", range.start, range.end - 1)
                    .parse()
                    .map_err(|_| Error::InvalidValue)?,
            );
        }
        Ok(request)
    }

    fn request(
        &self,
        method: Method,
        segments: &[&str],
        token: Option<&Token>,
    ) -> Result<Request, Error> {
        let mut url = self.base.clone();
        url.path_segments_mut()
            .map_err(|_| Error::InvalidServer)?
            .pop_if_empty()
            .extend(segments);
        let authorization = match token {
            Some(token) => format!("{}, Token=\"{}\"", self.authorization, token.0),
            None => self.authorization.clone(),
        };
        let mut authorization = reqwest::header::HeaderValue::from_str(&authorization)
            .map_err(|_| Error::InvalidValue)?;
        authorization.set_sensitive(true);
        self.http
            .request(method, url)
            .header(reqwest::header::AUTHORIZATION, authorization)
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|_| Error::InvalidValue)
    }

    async fn json<T: DeserializeOwned>(&self, request: Request) -> Result<T, Error> {
        let response = self
            .http
            .execute(request)
            .await
            .map_err(|_| Error::Network)?;
        if !response.status().is_success() {
            return Err(Error::Http(response.status().as_u16()));
        }
        response.json().await.map_err(|_| Error::InvalidResponse)
    }
}

fn with_json(mut request: Request, body: &serde_json::Value) -> Result<Request, Error> {
    request.headers_mut().insert(
        reqwest::header::CONTENT_TYPE,
        "application/json"
            .parse()
            .map_err(|_| Error::InvalidValue)?,
    );
    *request.body_mut() = Some(body.to_string().into());
    Ok(request)
}

fn normalize_server(input: &str) -> Result<Url, Error> {
    let input = input.trim();
    let address = if input.contains("://") {
        input.to_owned()
    } else {
        format!("http://{input}")
    };
    let mut url = Url::parse(&address).map_err(|_| Error::InvalidServer)?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(Error::InvalidServer);
    }
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    Ok(url)
}

fn validate_header_value(value: &str) -> Result<(), Error> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && !matches!(byte, b'"' | b'\\'))
    {
        return Err(Error::InvalidValue);
    }
    Ok(())
}

pub(crate) fn validate_id(value: &str) -> Result<(), Error> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(Error::InvalidValue);
    }
    Ok(())
}

#[cfg(test)]
mod tests;

pub mod browser;
