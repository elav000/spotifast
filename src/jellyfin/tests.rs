use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// A bounded local server, returning every request for contract assertions.
async fn server(
    replies: Vec<(u16, &'static str)>,
) -> (String, tokio::task::JoinHandle<Vec<String>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://{}/jellyfin", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for (status, body) in replies {
            let (mut stream, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let mut bytes = Vec::new();
            loop {
                let mut buffer = [0; 1024];
                let count = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buffer))
                    .await
                    .unwrap()
                    .unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&buffer[..count]);
                if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&bytes[..end]);
                    let length: usize = header
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse().unwrap())
                        })
                        .unwrap_or(0);
                    if bytes.len() >= end + 4 + length {
                        break;
                    }
                }
                assert!(bytes.len() < 64 * 1024);
            }
            requests.push(String::from_utf8(bytes).unwrap());
            let response = format!(
                "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\nLocation: http://127.0.0.1:1/leak\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        }
        requests
    });
    (address, task)
}

fn client(address: &str) -> Client {
    Client::new(address, "stable-device", &ProxyConfig::Off).unwrap()
}

fn token() -> Token {
    Token::new("private-token".into()).unwrap()
}

#[test]
fn normalizes_local_addresses_and_preserves_reverse_proxy_path() {
    assert_eq!(
        normalize_server(" nas.tailnet.ts.net:8096/jellyfin ")
            .unwrap()
            .as_str(),
        "http://nas.tailnet.ts.net:8096/jellyfin/"
    );
    assert_eq!(
        normalize_server("https://music.example/jellyfin/")
            .unwrap()
            .as_str(),
        "https://music.example/jellyfin/"
    );
    for invalid in [
        "",
        "https://user:password@host/",
        "https://host/?api_key=secret",
        "https://host/#secret",
        "file:///music",
        "ftp://host/",
        "http://",
    ] {
        assert_eq!(
            normalize_server(invalid),
            Err(Error::InvalidServer),
            "{invalid}"
        );
    }
}

#[test]
fn rejects_header_injection_and_path_traversal() {
    for invalid in ["", "x\r\nInjected: yes", "x\"", "x\\y", "a b"] {
        assert!(Token::new(invalid.into()).is_err());
        assert!(Client::new("http://localhost", invalid, &ProxyConfig::Off).is_err());
    }
    for invalid in ["", "../admin", "x/y", "x?secret=y", ".", "é"] {
        assert_eq!(validate_id(invalid), Err(Error::InvalidValue));
    }
}

#[tokio::test]
async fn quick_connect_keeps_device_identity_and_exchanges_approved_secret() {
    let (address, requests) = server(vec![
        (200, r#"{"Code":"123456","Secret":"temporary-secret"}"#),
        (200, r#"{"Authenticated":false}"#),
        (200, r#"{"Authenticated":true}"#),
        (
            200,
            r#"{"AccessToken":"private-token","ServerId":"server1","User":{"Id":"user1"}}"#,
        ),
    ])
    .await;
    let client = client(&address);
    let challenge = client.initiate().await.unwrap();
    assert_eq!(challenge.code, "123456");
    assert!(matches!(
        client.finish(&challenge).await,
        Err(Error::Pending)
    ));
    let account = client.finish(&challenge).await.unwrap();
    assert_eq!(account.user_id, "user1");
    assert_eq!(account.server_id, "server1");
    let requests = requests.await.unwrap();
    assert!(requests[0].starts_with("POST /jellyfin/QuickConnect/Initiate "));
    assert!(requests[1].starts_with("GET /jellyfin/QuickConnect/Connect?Secret=temporary-secret "));
    assert!(requests[3].starts_with("POST /jellyfin/Users/AuthenticateWithQuickConnect "));
    assert!(requests[3].ends_with(r#"{"Secret":"temporary-secret"}"#));
    for request in &requests {
        assert!(request.contains("DeviceId=\"stable-device\""));
        assert!(!request.contains("private-token"));
    }
}

#[tokio::test]
async fn library_and_album_requests_are_user_scoped_and_paged() {
    let (address, requests) = server(vec![
        (200, r#"{"Items":[{"Id":"music1","Name":"Music","CollectionType":"music"},{"Id":"video1","Name":"Movies","CollectionType":"movies"}],"TotalRecordCount":2,"StartIndex":0}"#),
        (200, r#"{"Items":[{"Id":"album1","Name":"Album"}],"TotalRecordCount":101,"StartIndex":100}"#),
        (200, r#"{"Items":[{"Id":"track1","Name":"Song","Artists":["Artist"],"RunTimeTicks":123450000}],"TotalRecordCount":1,"StartIndex":0}"#),
    ]).await;
    let client = client(&address);
    let libraries = client.music_libraries("user1", &token()).await.unwrap();
    assert_eq!(libraries.len(), 1);
    assert_eq!(libraries[0].id, "music1");
    let albums = client
        .albums("user1", "music1", 100, &token())
        .await
        .unwrap();
    assert_eq!(albums.total_record_count, 101);
    let tracks = client
        .album_tracks("user1", "album1", 0, &token())
        .await
        .unwrap();
    assert_eq!(tracks.items[0].duration_ms(), Some(12345));
    let requests = requests.await.unwrap();
    assert!(requests[0].starts_with("GET /jellyfin/UserViews?UserId=user1 "));
    for request in &requests {
        assert!(request.contains("Token=\"private-token\""));
        assert!(!request.lines().next().unwrap().contains("private-token"));
    }
    let url = Url::parse(&format!(
        "http://localhost{}",
        requests[1].split_whitespace().nth(1).unwrap()
    ))
    .unwrap();
    let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(query["UserId"], "user1");
    assert_eq!(query["ParentId"], "music1");
    assert_eq!(query["StartIndex"], "100");
    assert_eq!(query["Limit"], "100");
    assert_eq!(query["IncludeItemTypes"], "MusicAlbum");
    assert!(requests[2].contains("IncludeItemTypes=Audio"));
    assert!(requests[2].contains("SortBy=ParentIndexNumber%2CIndexNumber%2CSortName"));
}

#[tokio::test]
async fn errors_omit_secrets_and_redirects_are_not_followed() {
    for (status, body, expected) in [
        (401, "private-token", Error::Http(401)),
        (302, "temporary-secret", Error::Http(302)),
        (200, "invalid temporary-secret", Error::InvalidResponse),
    ] {
        let (address, requests) = server(vec![(status, body)]).await;
        let challenge = Challenge {
            code: "123456".into(),
            secret: "temporary-secret".into(),
        };
        let error = client(&address).approved(&challenge).await.unwrap_err();
        assert_eq!(error, expected);
        let shown = format!("{error} {error:?}");
        assert!(!shown.contains("secret"));
        assert!(!shown.contains("private-token"));
        assert!(!shown.contains(&address));
        assert_eq!(requests.await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn rejects_a_page_from_the_wrong_offset() {
    let (address, requests) = server(vec![(
        200,
        r#"{"Items":[],"TotalRecordCount":1000,"StartIndex":0}"#,
    )])
    .await;
    assert_eq!(
        client(&address)
            .albums("user1", "music1", 100, &token())
            .await,
        Err(Error::InvalidResponse)
    );
    requests.await.unwrap();
}

#[test]
fn original_stream_ranges_and_credentials_are_separate_from_identity() {
    let client = client("http://localhost:8096/jellyfin");
    let request = client
        .stream_request("track1", &token(), Some(256..512))
        .unwrap();
    assert_eq!(
        request.url().as_str(),
        "http://localhost:8096/jellyfin/Audio/track1/stream?static=true"
    );
    assert_eq!(request.headers()[reqwest::header::RANGE], "bytes=256-511");
    assert!(!format!("{request:?}").contains("private-token"));
    assert!(
        request.headers()[reqwest::header::AUTHORIZATION]
            .to_str()
            .unwrap()
            .contains("private-token")
    );
    assert!(
        client
            .stream_request("track1", &token(), Some(5..5))
            .is_err()
    );
    assert!(client.stream_request("../admin", &token(), None).is_err());
}

#[test]
fn media_details_round_trip_without_persisting_provider_urls_or_headers() {
    let item: Item = serde_json::from_value(serde_json::json!({
        "Id":"track1", "Container":"wav", "Artists":[],
        "MediaStreams":[{"Type":"Audio","Codec":"pcm_s24le","SampleRate":96000,"Channels":2,"BitDepth":24}],
        "MediaSources":[{"Container":"wav","RequiredHttpHeaders":{"Authorization":"secret-token"},"TranscodingUrl":"/stream?api_key=secret-token","Path":"private-server-path"}]
    })).unwrap();
    assert_eq!(
        item.media_streams.as_ref().unwrap()[0].codec.as_deref(),
        Some("pcm_s24le")
    );
    assert_eq!(item.media_streams.as_ref().unwrap()[0].bit_depth, Some(24));
    let cached = serde_json::to_string(&item).unwrap();
    assert!(!cached.contains("secret-token"));
    assert!(!cached.contains("private-server-path"));
    let old: Item = serde_json::from_str(r#"{"Id":"old-track"}"#).unwrap();
    assert!(old.container.is_none());
}
