//! A common audio-cache allowance with isolated backend stores.
//!
//! Librespot retains its own file format and eviction. HTTP range caching uses
//! the other half of the allowance, outside librespot's recursively scanned
//! directory. All operations here belong on media/decoder workers.
use sha1::{Digest, Sha1};
use std::{
    fs::{self, File, FileTimes},
    io::{self, Read},
    path::{Path, PathBuf},
    sync::Mutex,
    time::SystemTime,
};

const DEFAULT_LIMIT: u64 = 1024 * 1024 * 1024;
const MAX_RANGE: usize = 1024 * 1024;
const HEADER: usize = 5 + 8 + 20;
static DISK: Mutex<()> = Mutex::new(());

pub struct Policy {
    pub spotify_limit: Option<u64>,
    pub http: Option<RangeCache>,
}

impl Policy {
    pub fn new(audio_dir: Option<&Path>, limit: Option<u64>) -> Self {
        let Some(dir) = audio_dir else {
            return Self {
                spotify_limit: None,
                http: None,
            };
        };
        let limit = limit.unwrap_or(DEFAULT_LIMIT);
        let http_limit = limit / 2;
        Self {
            spotify_limit: Some(limit - http_limit),
            http: Some(RangeCache {
                root: dir.with_extension("streams"),
                limit: http_limit,
            }),
        }
    }
}

/// Stable provider identity, never a transport URL, token or Spotify-shaped ID.
#[derive(Clone, serde::Serialize)]
pub struct Identity {
    pub provider: String,
    pub source: String,
    pub account: String,
    pub item: String,
    /// Encoded variant, for example original or a specific transcoding profile.
    pub representation: String,
}

#[derive(Clone)]
pub struct Binding {
    cache: RangeCache,
    identity: Identity,
}

#[derive(Clone)]
pub struct RangeCache {
    root: PathBuf,
    limit: u64,
}

impl RangeCache {
    /// Open on a media worker so a reduced allowance also trims old ranges.
    pub fn bind(self, identity: Identity) -> Binding {
        {
            let _guard = DISK.lock().unwrap_or_else(|p| p.into_inner());
            let _ = self.prune();
        }
        Binding {
            cache: self,
            identity,
        }
    }
}

impl RangeCache {
    /// Caller holds DISK. Only this store's complete owned files are eligible.
    fn prune(&self) -> io::Result<()> {
        let mut files = Vec::new();
        let mut used = 0u64;
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str().and_then(|name| name.strip_suffix(".range")) else {
                continue;
            };
            if name.len() != 40 || !name.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                continue;
            }
            let meta = fs::symlink_metadata(entry.path())?;
            if !meta.is_file() {
                continue;
            }
            used = used.saturating_add(meta.len());
            files.push((
                meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                entry.path(),
                meta.len(),
            ));
        }
        files.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        for (_, file, size) in files {
            if used <= self.limit {
                break;
            }
            fs::remove_file(file)?;
            used = used.saturating_sub(size);
        }
        Ok(())
    }
}

impl Binding {
    fn path(&self, validator: &[u8], total: u64, start: u64, count: usize) -> PathBuf {
        let key = serde_json::to_vec(&(1, &self.identity, validator, total, start, count))
            .expect("serializable cache identity");
        self.cache
            .root
            .join(format!("{:x}.range", Sha1::digest(key)))
    }

    /// The caller first validates the current entity with the server. A validator
    /// and length from a prior launch alone are never enough to authorize a hit.
    pub fn read(&self, validator: &[u8], total: u64, start: u64, count: usize) -> Option<Vec<u8>> {
        if count == 0 || count > MAX_RANGE || (count + HEADER) as u64 > self.cache.limit {
            return None;
        }
        let _guard = DISK.lock().unwrap_or_else(|p| p.into_inner());
        let path = self.path(validator, total, start, count);
        let metadata = fs::symlink_metadata(&path).ok()?;
        if !metadata.is_file() || metadata.len() != (count + HEADER) as u64 {
            return None;
        }
        let mut data = Vec::with_capacity(count + HEADER);
        File::open(&path)
            .ok()?
            .take((count + HEADER + 1) as u64)
            .read_to_end(&mut data)
            .ok()?;
        if data.len() != count + HEADER
            || &data[..5] != b"SFAC1"
            || u64::from_le_bytes(data[5..13].try_into().ok()?) != count as u64
            || data[13..HEADER] != Sha1::digest(&data[HEADER..])[..]
        {
            return None;
        }
        // Windows needs write access to update file timestamps. Opening the
        // existing cache file this way does not truncate its contents.
        if let Ok(file) = File::options().write(true).open(path) {
            let _ = file.set_times(FileTimes::new().set_modified(SystemTime::now()));
        }
        Some(data[HEADER..].to_vec())
    }

    /// Disposable cache failures never fail playback. Publish only complete,
    /// validated ranges, atomically; eviction stays inside this backend store.
    pub fn write(&self, validator: &[u8], total: u64, start: u64, bytes: &[u8]) -> io::Result<()> {
        if bytes.is_empty()
            || bytes.len() > MAX_RANGE
            || (bytes.len() + HEADER) as u64 > self.cache.limit
        {
            return Ok(());
        }
        let _guard = DISK.lock().unwrap_or_else(|p| p.into_inner());
        let path = self.path(validator, total, start, bytes.len());
        fs::create_dir_all(&self.cache.root)?;
        let mut data = Vec::with_capacity(bytes.len() + HEADER);
        data.extend_from_slice(b"SFAC1");
        data.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        data.extend_from_slice(&Sha1::digest(bytes));
        data.extend_from_slice(bytes);
        fastframe_file::write(&path, data)?;
        self.cache.prune()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn binding(name: &str, limit: u64) -> Binding {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/media-cache-tests")
            .join(format!("{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        RangeCache { root, limit }.bind(Identity {
            provider: "http-provider".into(),
            source: "server".into(),
            account: "account".into(),
            item: "song".into(),
            representation: "original".into(),
        })
    }
    #[test]
    fn policy_disables_both_caches_or_bounds_the_combined_allowance() {
        assert!(Policy::new(None, Some(100)).http.is_none());
        let policy = Policy::new(Some(Path::new("cache/audio")), Some(101));
        assert_eq!(policy.spotify_limit, Some(51));
        assert_eq!(policy.http.as_ref().unwrap().limit, 50);
        assert_eq!(policy.http.unwrap().root, Path::new("cache/audio.streams"));
    }
    #[test]
    fn cached_bytes_are_isolated_by_account_source_item_provider_and_entity() {
        let binding = binding("identity", 4096);
        binding.write(b"etag", 1000, 0, &[7; 100]).unwrap();
        assert_eq!(binding.read(b"etag", 1000, 0, 100), Some(vec![7; 100]));
        for field in 0..5 {
            let mut other = binding.clone();
            match field {
                0 => other.identity.account.push('x'),
                1 => other.identity.source.push('x'),
                2 => other.identity.item.push('x'),
                3 => other.identity.provider.push('x'),
                _ => other.identity.representation.push('x'),
            }
            assert!(other.read(b"etag", 1000, 0, 100).is_none());
        }
        assert!(binding.read(b"changed", 1000, 0, 100).is_none());
        assert!(binding.read(b"etag", 1001, 0, 100).is_none());
        assert!(binding.read(b"etag", 1000, 1, 100).is_none());
        let _ = fs::remove_dir_all(binding.cache.root);
    }
    #[test]
    fn reopening_with_a_smaller_allowance_prunes_only_owned_ranges() {
        let binding = binding("reopen", 4096);
        binding.write(b"etag", 1000, 0, &[7; 100]).unwrap();
        let unrelated = binding.cache.root.join("keep.txt");
        fs::write(&unrelated, b"keep").unwrap();
        let mut reduced = binding.cache.clone();
        reduced.limit = 0;
        let reduced = reduced.bind(binding.identity.clone());
        assert!(reduced.read(b"etag", 1000, 0, 100).is_none());
        assert_eq!(fs::read(unrelated).unwrap(), b"keep");
        assert_eq!(fs::read_dir(&reduced.cache.root).unwrap().count(), 1);
        let _ = fs::remove_dir_all(reduced.cache.root);
    }
    #[test]
    fn corrupt_ranges_miss_and_eviction_keeps_the_store_bounded() {
        let binding = binding("eviction", (HEADER + 100) as u64);
        binding.write(b"etag", 1000, 0, &[1; 100]).unwrap();
        let path = binding.path(b"etag", 1000, 0, 100);
        let mut corrupt = fs::read(&path).unwrap();
        corrupt[HEADER] ^= 1;
        fs::write(&path, corrupt).unwrap();
        assert!(binding.read(b"etag", 1000, 0, 100).is_none());
        // Mark the old entry unambiguously older than its replacement.
        File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(FileTimes::new().set_modified(SystemTime::UNIX_EPOCH))
            .unwrap();
        binding.write(b"etag", 1000, 100, &[2; 100]).unwrap();
        assert!(binding.read(b"etag", 1000, 0, 100).is_none());
        assert_eq!(binding.read(b"etag", 1000, 100, 100), Some(vec![2; 100]));
        let size: u64 = fs::read_dir(&binding.cache.root)
            .unwrap()
            .map(|e| e.unwrap().metadata().unwrap().len())
            .sum();
        assert!(size <= binding.cache.limit);
        let _ = fs::remove_dir_all(binding.cache.root);
    }

    #[test]
    fn a_cache_hit_refreshes_its_eviction_timestamp_without_changing_bytes() {
        let binding = binding("touch", 4096);
        binding.write(b"etag", 1000, 0, &[7; 100]).unwrap();
        let path = binding.path(b"etag", 1000, 0, 100);
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(FileTimes::new().set_modified(SystemTime::UNIX_EPOCH))
            .unwrap();
        assert_eq!(binding.read(b"etag", 1000, 0, 100), Some(vec![7; 100]));
        assert!(fs::metadata(path).unwrap().modified().unwrap() > SystemTime::UNIX_EPOCH);
        let _ = fs::remove_dir_all(binding.cache.root);
    }
}
