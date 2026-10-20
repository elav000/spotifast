use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[test]
fn audio_format_uses_reported_audio_stream_metadata() {
    let item: Item = serde_json::from_str(
        r#"{"Id":"song","Name":"Song","MediaStreams":[{"Type":"Video","SampleRate":24000},{"Type":"Audio","SampleRate":48000,"BitRate":320000}],"MediaSources":[{"MediaStreams":[{"Type":"Audio","SampleRate":96000,"BitRate":1000000}]}]}"#,
    )
    .unwrap();
    assert_eq!(
        item.audio_format(),
        Some(crate::app::AudioFormat {
            bitrate_kbps: Some(320),
            sample_rate_hz: Some(48_000),
        })
    );
}

#[test]
fn audio_format_falls_back_to_media_source_and_omits_unknown_values() {
    let item: Item = serde_json::from_str(
        r#"{"Id":"song","Name":"Song","MediaSources":[{"MediaStreams":[{"Type":"Audio","SampleRate":96000}]}]}"#,
    )
    .unwrap();
    assert_eq!(
        item.audio_format(),
        Some(crate::app::AudioFormat {
            bitrate_kbps: None,
            sample_rate_hz: Some(96_000),
        })
    );
    let item: Item = serde_json::from_str(r#"{"Id":"song","Name":"Song"}"#).unwrap();
    assert_eq!(item.audio_format(), None);
}

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

#[tokio::test]
async fn primary_artwork_preserves_proxy_path_and_keeps_credentials_in_headers() {
    let (address, requests) = server(vec![(200, "image-bytes")]).await;
    let response = client(&address)
        .primary_image("album1", "cover-v2", &token())
        .await
        .unwrap();
    assert_eq!(response.bytes().await.unwrap().as_ref(), b"image-bytes");
    let requests = requests.await.unwrap();
    let request = &requests[0];
    let first_line = request.lines().next().unwrap();
    assert!(first_line.contains("/jellyfin/Items/album1/Images/Primary?Tag=cover-v2&MaxWidth=640"));
    assert!(!first_line.contains("private-token"));
    assert!(
        request
            .to_lowercase()
            .contains("authorization: mediabrowser")
    );
    assert!(request.contains("Token=\"private-token\""));
    assert!(request.to_lowercase().contains("accept: image/*"));
}

#[tokio::test]
async fn primary_artwork_does_not_follow_redirects() {
    let (address, requests) = server(vec![(302, "")]).await;
    let error = client(&address)
        .primary_image("album1", "cover-v2", &token())
        .await
        .unwrap_err();
    assert_eq!(error, Error::Http(302));
    assert_eq!(requests.await.unwrap().len(), 1);
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

#[tokio::test]
async fn provider_catalog_queries_are_scoped_paged_and_preserve_playlist_occurrences() {
    use super::catalog::{Kind, Query};
    const EMPTY: &str = r#"{"Items":[],"StartIndex":0,"TotalRecordCount":0}"#;
    const PLAYLIST: &str = r#"{"Items":[{"Id":"song1","PlaylistItemId":"entry1","Type":"Audio"},{"Id":"song1","PlaylistItemId":"entry2","Type":"Audio"}],"StartIndex":100,"TotalRecordCount":102}"#;
    let (address, requests) = server(vec![
        (200, EMPTY),
        (200, EMPTY),
        (200, EMPTY),
        (200, PLAYLIST),
        (200, EMPTY),
        (200, EMPTY),
        (200, EMPTY),
    ])
    .await;
    let client = client(&address);
    let token = token();
    for query in [
        Query::Shelf {
            library: "music1".into(),
            kind: Kind::Artist,
        },
        Query::Shelf {
            library: "music1".into(),
            kind: Kind::Playlist,
        },
        Query::ArtistAlbums {
            library: "music1".into(),
            artist: "artist1".into(),
        },
    ] {
        client.query("user1", &query, 0, &token).await.unwrap();
    }
    let page = client
        .query(
            "user1",
            &Query::PlaylistSongs {
                playlist: "playlist1".into(),
            },
            100,
            &token,
        )
        .await
        .unwrap();
    assert_eq!(page.items[0].id, page.items[1].id);
    assert_ne!(
        page.items[0].details.playlist_item_id,
        page.items[1].details.playlist_item_id
    );
    client
        .query(
            "user1",
            &Query::Search {
                library: "music1".into(),
                term: "BT & friends".into(),
                kind: Kind::Song,
            },
            0,
            &token,
        )
        .await
        .unwrap();
    client
        .query(
            "user1",
            &Query::History {
                library: "music1".into(),
            },
            0,
            &token,
        )
        .await
        .unwrap();
    client
        .query("user1", &Query::Recent, 0, &token)
        .await
        .unwrap();
    let requests = requests.await.unwrap();
    let urls: Vec<_> = requests
        .iter()
        .map(|request| {
            let path = request
                .lines()
                .next()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap();
            assert!(!path.contains("private-token"));
            assert!(request.contains("Token=\"private-token\""));
            Url::parse(&format!("http://test{path}")).unwrap()
        })
        .collect();
    assert_eq!(urls[0].path(), "/jellyfin/Artists");
    assert!(
        urls[0]
            .query_pairs()
            .any(|(k, v)| k == "ParentId" && v == "music1")
    );
    assert!(!urls[1].query_pairs().any(|(k, _)| k == "ParentId"));
    assert!(
        urls[2]
            .query_pairs()
            .any(|(k, v)| k == "ArtistIds" && v == "artist1")
    );
    assert_eq!(urls[3].path(), "/jellyfin/Playlists/playlist1/Items");
    assert!(!urls[3].query_pairs().any(|(k, _)| k == "SortBy"));
    assert!(
        urls[4]
            .query_pairs()
            .any(|(k, v)| k == "SearchTerm" && v == "BT & friends")
    );
    assert!(
        urls[5]
            .query_pairs()
            .any(|(k, v)| k == "SortBy" && v == "DatePlayed")
    );
    assert!(
        urls[5]
            .query_pairs()
            .any(|(k, v)| k == "Filters" && v == "IsPlayed")
    );
    assert_eq!(urls[6].path(), "/jellyfin/Items");
    assert!(
        urls[6]
            .query_pairs()
            .any(|(k, v)| k == "IncludeItemTypes" && v == "Audio")
    );
    assert!(
        urls[6]
            .query_pairs()
            .any(|(k, v)| k == "SortBy" && v == "DatePlayed")
    );
    assert!(
        urls[6]
            .query_pairs()
            .any(|(k, v)| k == "Filters" && v == "IsPlayed")
    );
    assert!(!urls[6].query_pairs().any(|(key, _)| key == "ParentId"));
}

#[tokio::test]
async fn playlist_mutations_and_playback_reports_keep_credentials_out_of_urls() {
    use super::catalog::PlaybackEvent;
    let (address, requests) = server(vec![
        (200, r#"{"Id":"playlist1"}"#),
        (204, ""),
        (204, ""),
        (204, ""),
        (204, ""),
        (204, ""),
        (204, ""),
    ])
    .await;
    let client = client(&address);
    let token = token();
    assert_eq!(
        client
            .create_playlist("user1", "A private mix", &token)
            .await
            .unwrap(),
        "playlist1"
    );
    client
        .rename_playlist("playlist1", "Renamed", &token)
        .await
        .unwrap();
    client
        .add_to_playlist(
            "user1",
            "playlist1",
            &["song1".into(), "song1".into()],
            &token,
        )
        .await
        .unwrap();
    client
        .remove_playlist_entries("playlist1", &["entry2".into()], &token)
        .await
        .unwrap();
    client
        .move_playlist_entry("playlist1", "entry1", 5, &token)
        .await
        .unwrap();
    client
        .report_playback("song1", 1234, false, PlaybackEvent::Progress, &token)
        .await
        .unwrap();
    client.delete_playlist("playlist1", &token).await.unwrap();
    let requests = requests.await.unwrap();
    assert!(requests[0].contains("\"IsPublic\":false"));
    assert!(
        requests[1].ends_with(r#"{"Name":"Renamed"}"#),
        "rename must not replace playlist entries or sharing"
    );
    assert!(
        requests[2]
            .lines()
            .next()
            .unwrap()
            .contains("Ids=song1%2Csong1")
    );
    assert!(
        requests[3]
            .lines()
            .next()
            .unwrap()
            .contains("EntryIds=entry2")
    );
    assert!(requests[4].starts_with("POST /jellyfin/Playlists/playlist1/Items/entry1/Move/5 "));
    assert!(requests[5].contains("\"PositionTicks\":12340000"));
    assert!(requests[6].starts_with("DELETE /jellyfin/Items/playlist1 "));
    for request in &requests {
        assert!(!request.lines().next().unwrap().contains("private-token"));
    }
}

#[tokio::test]
async fn provider_catalog_and_mutations_reject_invalid_input_before_network() {
    use super::catalog::{Kind, PlaybackEvent, Query};
    let client = client("http://127.0.0.1:1/jellyfin");
    let token = token();
    for query in [
        Query::Shelf {
            library: "../other".into(),
            kind: Kind::Album,
        },
        Query::Search {
            library: "music1".into(),
            term: "\r\n".into(),
            kind: Kind::Song,
        },
    ] {
        assert_eq!(
            client.query("user1", &query, 0, &token).await.unwrap_err(),
            Error::InvalidValue
        );
    }
    assert_eq!(
        client
            .remove_playlist_entries("playlist1", &[], &token)
            .await
            .unwrap_err(),
        Error::InvalidValue
    );
    assert_eq!(
        client
            .create_playlist("user1", " ", &token)
            .await
            .unwrap_err(),
        Error::InvalidValue
    );
    assert_eq!(
        client
            .report_playback("song1", u64::MAX, false, PlaybackEvent::Start, &token)
            .await
            .unwrap_err(),
        Error::InvalidValue
    );
}

#[tokio::test]
async fn artist_songs_stay_in_the_selected_collection() {
    use super::catalog::Query;
    let (address, requests) = server(vec![(
        200,
        r#"{"Items":[],"StartIndex":0,"TotalRecordCount":0}"#,
    )])
    .await;
    client(&address)
        .query(
            "user1",
            &Query::ArtistSongs {
                library: "music2".into(),
                artist: "artist1".into(),
            },
            0,
            &token(),
        )
        .await
        .unwrap();
    let requests = requests.await.unwrap();
    let path = requests[0].split_whitespace().nth(1).unwrap();
    let url = Url::parse(&format!("http://test{path}")).unwrap();
    for (key, value) in [
        ("ParentId", "music2"),
        ("ArtistIds", "artist1"),
        ("IncludeItemTypes", "Audio"),
        ("UserId", "user1"),
    ] {
        assert!(url.query_pairs().any(|(k, v)| k == key && v == value));
    }
}

#[tokio::test]
async fn catalog_rejects_mismatched_and_nonadvancing_pages() {
    use super::catalog::{Kind, Query};
    let (address, requests) = server(vec![
        (200, r#"{"Items":[],"StartIndex":0,"TotalRecordCount":101}"#),
        (
            200,
            r#"{"Items":[],"StartIndex":100,"TotalRecordCount":101}"#,
        ),
    ])
    .await;
    let client = client(&address);
    for _ in 0..2 {
        assert_eq!(
            client
                .query(
                    "user1",
                    &Query::Shelf {
                        library: "music1".into(),
                        kind: Kind::Artist,
                    },
                    100,
                    &token()
                )
                .await
                .unwrap_err(),
            Error::InvalidResponse
        );
    }
    assert_eq!(requests.await.unwrap().len(), 2);
}

#[tokio::test]
async fn playlist_batch_stops_after_failure_and_validates_all_ids_first() {
    let (address, requests) = server(vec![(204, ""), (403, "")]).await;
    let client = client(&address);
    let ids: Vec<_> = (0..201).map(|index| format!("song{index}")).collect();
    assert_eq!(
        client
            .add_to_playlist("user1", "playlist1", &ids, &token())
            .await
            .unwrap_err(),
        Error::Http(403)
    );
    let requests = requests.await.unwrap();
    assert_eq!(requests.len(), 2);
    for (index, request) in requests.iter().enumerate() {
        let path = request.split_whitespace().nth(1).unwrap();
        let url = Url::parse(&format!("http://test{path}")).unwrap();
        let actual = url
            .query_pairs()
            .find(|(key, _)| key == "Ids")
            .unwrap()
            .1
            .into_owned();
        assert_eq!(actual, ids[index * 100..(index + 1) * 100].join(","));
    }
    let mut invalid = ids;
    invalid[200] = "../bad".into();
    assert_eq!(
        client
            .add_to_playlist("user1", "playlist1", &invalid, &token())
            .await
            .unwrap_err(),
        Error::InvalidValue
    );
}

#[tokio::test]
async fn creating_a_playlist_includes_initial_occurrences_in_one_request() {
    let (address, requests) = server(vec![(200, r#"{"Id":"playlist1"}"#)]).await;
    assert_eq!(
        client(&address)
            .create_playlist_with_items(
                "user1",
                "New mix",
                true,
                &["song1".into(), "song1".into()],
                &token()
            )
            .await
            .unwrap(),
        "playlist1"
    );
    let requests = requests.await.unwrap();
    assert_eq!(requests.len(), 1);
    let body: serde_json::Value =
        serde_json::from_str(requests[0].split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(body["Ids"], serde_json::json!(["song1", "song1"]));
    assert_eq!(body["IsPublic"], true);
}
