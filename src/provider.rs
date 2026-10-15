//! Browsing selection only. Playback ownership is intentionally independent.
//! Each provider owns its catalog; selecting one never merges libraries.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Provider {
    Spotify,
    #[default]
    Jellyfin,
}
