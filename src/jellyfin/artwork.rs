//! Stable artwork identities and the Jellyfin adapter for the shared ArtLoader.
use super::{Item, browser::Source};
use crate::{
    images::{ArtworkProvider, ArtworkResponse},
    paths::AppDirs,
    settings::ProxyConfig,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
    pub source: String,
    pub account: String,
    pub item: String,
    pub tag: String,
}

impl Reference {
    pub fn for_item(source: &Source, item: &Item) -> Option<Self> {
        let (id, tag) = item
            .image_tags
            .as_ref()
            .and_then(|tags| tags.get("Primary"))
            .and_then(Option::as_deref)
            .map(|tag| (item.id.as_str(), tag))
            .or_else(|| {
                Some((
                    item.album_id.as_deref()?,
                    item.album_primary_image_tag.as_deref()?,
                ))
            })?;
        let reference = Self {
            source: source.id.clone(),
            account: source.user_id.clone(),
            item: id.to_owned(),
            tag: tag.to_owned(),
        };
        Self::parse(&reference.uri())
    }

    pub fn uri(&self) -> String {
        format!(
            "provider-art://jellyfin/{}/{}/{}/{}",
            self.source, self.account, self.item, self.tag
        )
    }

    pub fn parse(uri: &str) -> Option<Self> {
        let parts: Vec<_> = uri
            .strip_prefix("provider-art://jellyfin/")?
            .split('/')
            .collect();
        if parts.len() != 4
            || parts[0].len() != 32
            || !parts[0].bytes().all(|byte| byte.is_ascii_hexdigit())
            || parts.iter().any(|part| super::validate_id(part).is_err())
        {
            return None;
        }
        Some(Self {
            source: parts[0].into(),
            account: parts[1].into(),
            item: parts[2].into(),
            tag: parts[3].into(),
        })
    }
}

pub struct Fetcher {
    dirs: AppDirs,
}

impl Fetcher {
    pub fn new(dirs: AppDirs) -> Self {
        Self { dirs }
    }
}

impl ArtworkProvider for Fetcher {
    fn response(&self, uri: String, proxy: ProxyConfig) -> ArtworkResponse {
        let dirs = self.dirs.clone();
        Box::pin(async move {
            let reference = Reference::parse(&uri).ok_or("Invalid Jellyfin artwork reference.")?;
            let identity = reference.clone();
            let (client, token) = tokio::task::spawn_blocking(move || {
                let source = crate::backend::jellyfin::saved_playback_source(&dirs)?;
                if source.id != identity.source || source.user_id != identity.account {
                    return Err("The Jellyfin artwork account changed.".to_owned());
                }
                crate::backend::jellyfin::saved_playback_session(&dirs, &source, &proxy)
            })
            .await
            .map_err(|_| "Could not load Jellyfin artwork credentials.")??;
            client
                .primary_image(&reference.item, &reference.tag, &token)
                .await
                .map_err(|error| error.to_string())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn artwork_cannot_load_credentials_for_a_different_account_or_revoked_source() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!("art-account-check-{}", std::process::id()));
        let dirs = AppDirs {
            config: root.join("config"),
            state: root.join("state"),
            cache: root.join("cache"),
        };
        std::fs::create_dir_all(&dirs.state).unwrap();
        let source = Source {
            id: "a".repeat(32),
            server_url: "http://127.0.0.1:1/".into(),
            server_id: "server1".into(),
            user_id: "user1".into(),
            device_id: "device1".into(),
        };
        std::fs::write(
            dirs.state.join("jellyfin-account.json"),
            serde_json::to_vec(&source).unwrap(),
        )
        .unwrap();
        let fetcher = Fetcher::new(dirs.clone());
        let mut reference = Reference {
            source: source.id.clone(),
            account: "user2".into(),
            item: "album1".into(),
            tag: "tag1".into(),
        };
        let error = fetcher
            .response(reference.uri(), ProxyConfig::Off)
            .await
            .unwrap_err();
        assert_eq!(error, "The Jellyfin artwork account changed.");
        reference.account = source.user_id.clone();
        std::fs::write(dirs.state.join("jellyfin-revoked"), &source.id).unwrap();
        let error = fetcher
            .response(reference.uri(), ProxyConfig::Off)
            .await
            .unwrap_err();
        assert_eq!(error, "Saved Jellyfin sign-in was revoked.");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn album_fallback_and_primary_tags_share_stable_artwork_identity() {
        let source: Source = serde_json::from_value(serde_json::json!({
            "id": "a".repeat(32), "server_url": "http://127.0.0.1/", "server_id":"server1", "user_id":"user1", "device_id":"device1"
        })).unwrap();
        let mut item: Item = serde_json::from_value(serde_json::json!({
            "Id":"track1", "AlbumId":"album1", "AlbumPrimaryImageTag":"cover-v2"
        }))
        .unwrap();
        let album = Reference::for_item(&source, &item).unwrap();
        assert_eq!(album.item, "album1");
        item.image_tags
            .get_or_insert_default()
            .insert("Primary".into(), Some("track-cover".into()));
        let track = Reference::for_item(&source, &item).unwrap();
        assert_eq!(track.item, "track1");
        assert_eq!(track.tag, "track-cover");
        assert!(!track.uri().contains("127.0.0.1"));
    }

    #[test]
    fn artwork_identity_is_source_account_item_and_revision_scoped() {
        let reference = Reference {
            source: "a".repeat(32),
            account: "user1".into(),
            item: "album1".into(),
            tag: "cover-v2".into(),
        };
        assert_eq!(Reference::parse(&reference.uri()), Some(reference.clone()));
        for bad in [
            reference.uri() + "/extra",
            reference.uri() + "?api_key=secret",
            reference.uri().replace("album1", ".."),
            reference.uri().replace("user1", "other/account"),
        ] {
            assert!(Reference::parse(&bad).is_none());
        }
        let mut revised = reference.clone();
        revised.tag = "cover-v3".into();
        assert_ne!(reference.uri(), revised.uri());
        revised = reference.clone();
        revised.account = "user2".into();
        assert_ne!(reference.uri(), revised.uri());
    }
}
