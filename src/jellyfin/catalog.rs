//! Account-scoped catalog, playlist and history operations for provider adapters.
use super::{Client, Error, Item, Page, Token, validate_id, with_json};
use reqwest::Method;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Kind {
    #[default]
    Album,
    Artist,
    Playlist,
    Song,
}

impl Kind {
    pub fn item_type(self) -> &'static str {
        match self {
            Self::Album => "MusicAlbum",
            Self::Artist => "MusicArtist",
            Self::Playlist => "Playlist",
            Self::Song => "Audio",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Query {
    Shelf {
        library: String,
        kind: Kind,
    },
    ArtistAlbums {
        library: String,
        artist: String,
    },
    ArtistSongs {
        library: String,
        artist: String,
    },
    PlaylistSongs {
        playlist: String,
    },
    Search {
        library: String,
        term: String,
        kind: Kind,
    },
    History {
        library: String,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Details {
    #[serde(rename = "Type")]
    pub kind: Option<String>,
    pub user_data: Option<UserData>,
    pub playlist_item_id: Option<String>,
    pub overview: Option<String>,
    pub date_created: Option<String>,
    pub can_delete: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UserData {
    pub last_played_date: Option<String>,
    pub play_count: Option<u32>,
    pub is_favorite: Option<bool>,
}

#[derive(Clone, Copy, Debug)]
pub enum PlaybackEvent {
    Start,
    Progress,
    Stop,
}

impl Client {
    pub async fn query(
        &self,
        user: &str,
        query: &Query,
        start: u32,
        token: &Token,
    ) -> Result<Page<Item>, Error> {
        validate_id(user)?;
        let mut artist_path = false;
        let mut parent = None;
        let mut artist = None;
        let mut playlist = None;
        let mut search = None;
        let mut history = false;
        let kind = match query {
            Query::Shelf { library, kind } => {
                parent = Some(library.as_str());
                *kind
            }
            Query::ArtistAlbums {
                library,
                artist: id,
            } => {
                parent = Some(library.as_str());
                artist = Some(id.as_str());
                Kind::Album
            }
            Query::ArtistSongs {
                library,
                artist: id,
            } => {
                parent = Some(library.as_str());
                artist = Some(id.as_str());
                Kind::Song
            }
            Query::PlaylistSongs { playlist: id } => {
                playlist = Some(id.as_str());
                Kind::Song
            }
            Query::Search {
                library,
                term,
                kind,
            } => {
                if term.trim().is_empty()
                    || term.chars().count() > 256
                    || term.chars().any(char::is_control)
                {
                    return Err(Error::InvalidValue);
                }
                parent = Some(library.as_str());
                search = Some(term.as_str());
                *kind
            }
            Query::History { library } => {
                parent = Some(library.as_str());
                history = true;
                Kind::Song
            }
        };
        for id in parent.into_iter().chain(artist).chain(playlist) {
            validate_id(id)?;
        }
        if kind == Kind::Artist {
            artist_path = true;
        }
        let segments = if let Some(id) = playlist {
            vec!["Playlists", id, "Items"]
        } else if artist_path {
            vec!["Artists"]
        } else {
            vec!["Items"]
        };
        let mut request = self.request(Method::GET, &segments, Some(token))?;
        let mut params = request.url_mut().query_pairs_mut();
        params
            .append_pair("UserId", user)
            .append_pair("StartIndex", &start.to_string())
            .append_pair("Limit", "100")
            .append_pair("EnableTotalRecordCount", "true")
            .append_pair("EnableUserData", "true")
            .append_pair(
                "Fields",
                "Artists,AlbumId,MediaStreams,MediaSources,Overview,DateCreated",
            );
        if playlist.is_none() {
            params
                .append_pair("Recursive", "true")
                .append_pair("SortBy", if history { "DatePlayed" } else { "SortName" })
                .append_pair(
                    "SortOrder",
                    if history { "Descending" } else { "Ascending" },
                );
            if !artist_path {
                params.append_pair("IncludeItemTypes", kind.item_type());
            }
            // Server playlists are user-scoped, not children of a music library.
            if kind != Kind::Playlist
                && let Some(parent) = parent
            {
                params.append_pair("ParentId", parent);
            }
            if let Some(artist) = artist {
                params.append_pair("ArtistIds", artist);
            }
            if let Some(term) = search {
                params.append_pair("SearchTerm", term);
            }
            if history {
                params.append_pair("Filters", "IsPlayed");
            }
        }
        drop(params);
        let page: Page<Item> = self.json(request).await?;
        if page.start_index != start
            || page.items.len() > 100
            || (page.items.is_empty() && start < page.total_record_count)
        {
            return Err(Error::InvalidResponse);
        }
        Ok(page)
    }

    pub async fn create_playlist(
        &self,
        user: &str,
        name: &str,
        token: &Token,
    ) -> Result<String, Error> {
        validate_id(user)?;
        validate_name(name)?;
        #[derive(Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct Created {
            id: String,
        }
        let request = with_json(
            self.request(Method::POST, &["Playlists"], Some(token))?,
            &serde_json::json!({"Name":name,"UserId":user,"MediaType":"Audio","IsPublic":false}),
        )?;
        let created: Created = self.json(request).await?;
        validate_id(&created.id)?;
        Ok(created.id)
    }

    pub async fn rename_playlist(
        &self,
        playlist: &str,
        name: &str,
        token: &Token,
    ) -> Result<(), Error> {
        validate_id(playlist)?;
        validate_name(name)?;
        let request = with_json(
            self.request(Method::POST, &["Playlists", playlist], Some(token))?,
            &serde_json::json!({"Name":name}),
        )?;
        self.catalog_success(request).await
    }

    pub async fn delete_playlist(&self, playlist: &str, token: &Token) -> Result<(), Error> {
        validate_id(playlist)?;
        self.catalog_success(self.request(Method::DELETE, &["Items", playlist], Some(token))?)
            .await
    }

    pub async fn add_to_playlist(
        &self,
        user: &str,
        playlist: &str,
        items: &[String],
        token: &Token,
    ) -> Result<(), Error> {
        validate_id(user)?;
        validate_id(playlist)?;
        validate_ids(items)?;
        for chunk in items.chunks(100) {
            let mut request =
                self.request(Method::POST, &["Playlists", playlist, "Items"], Some(token))?;
            request
                .url_mut()
                .query_pairs_mut()
                .append_pair("UserId", user)
                .append_pair("Ids", &chunk.join(","));
            self.catalog_success(request).await?;
        }
        Ok(())
    }

    pub async fn remove_playlist_entries(
        &self,
        playlist: &str,
        entries: &[String],
        token: &Token,
    ) -> Result<(), Error> {
        validate_id(playlist)?;
        validate_ids(entries)?;
        for chunk in entries.chunks(100) {
            let mut request = self.request(
                Method::DELETE,
                &["Playlists", playlist, "Items"],
                Some(token),
            )?;
            request
                .url_mut()
                .query_pairs_mut()
                .append_pair("EntryIds", &chunk.join(","));
            self.catalog_success(request).await?;
        }
        Ok(())
    }

    pub async fn move_playlist_entry(
        &self,
        playlist: &str,
        entry: &str,
        index: u32,
        token: &Token,
    ) -> Result<(), Error> {
        validate_id(playlist)?;
        validate_id(entry)?;
        self.catalog_success(self.request(
            Method::POST,
            &[
                "Playlists",
                playlist,
                "Items",
                entry,
                "Move",
                &index.to_string(),
            ],
            Some(token),
        )?)
        .await
    }

    pub async fn report_playback(
        &self,
        item: &str,
        position_ms: u64,
        paused: bool,
        event: PlaybackEvent,
        token: &Token,
    ) -> Result<(), Error> {
        validate_id(item)?;
        let ticks = position_ms
            .checked_mul(10_000)
            .and_then(|ticks| i64::try_from(ticks).ok())
            .ok_or(Error::InvalidValue)?;
        let path = match event {
            PlaybackEvent::Start => vec!["Sessions", "Playing"],
            PlaybackEvent::Progress => vec!["Sessions", "Playing", "Progress"],
            PlaybackEvent::Stop => vec!["Sessions", "Playing", "Stopped"],
        };
        let request = with_json(
            self.request(Method::POST, &path, Some(token))?,
            &serde_json::json!({"ItemId":item,"PositionTicks":ticks,"IsPaused":paused,"CanSeek":true,"PlayMethod":"DirectPlay"}),
        )?;
        self.catalog_success(request).await
    }

    async fn catalog_success(&self, request: reqwest::Request) -> Result<(), Error> {
        let response = self
            .http
            .execute(request)
            .await
            .map_err(|_| Error::Network)?;
        if !response.status().is_success() {
            return Err(Error::Http(response.status().as_u16()));
        }
        Ok(())
    }
}

fn validate_name(name: &str) -> Result<(), Error> {
    if name.trim().is_empty() || name.chars().count() > 256 || name.chars().any(char::is_control) {
        return Err(Error::InvalidValue);
    }
    Ok(())
}
fn validate_ids(ids: &[String]) -> Result<(), Error> {
    if ids.is_empty() {
        return Err(Error::InvalidValue);
    }
    for id in ids {
        validate_id(id)?;
    }
    Ok(())
}
