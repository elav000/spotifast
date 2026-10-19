//! Jellyfin backend worker. Owns network, protected credentials and disk work.
use keyring_core::api::CredentialStoreApi;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
    mpsc,
};

use crate::{
    backend::Waker,
    jellyfin::{Account, Challenge, Client, Token, browser::*},
    paths::AppDirs,
};

const SERVICE: &str = "rocks.spotifast.Spotifast.jellyfin";

pub fn spawn(
    dirs: AppDirs,
    waker: Waker,
    events: mpsc::Sender<Event>,
    generation: Arc<AtomicU64>,
    catalog_generation: Arc<AtomicU64>,
) -> mpsc::Sender<Job> {
    let (tx, rx) = mpsc::channel::<Job>();
    std::thread::Builder::new()
        .name("spotifast-jellyfin".into())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build();
            let Ok(runtime) = runtime else {
                return;
            };
            let mut worker = Worker {
                dirs,
                events,
                generation,
                catalog_generation,
                catalog_request: None,
                waker,
                source: None,
                servers: Vec::new(),
                token: None,
                challenge: None,
                pending_source: None,
                pending_client: None,
                client: None,
            };
            while let Ok(job) = rx.recv() {
                worker.catalog_request = job.catalog_request;
                if !worker.current(job.generation)
                    && !matches!(
                        &job.command,
                        Command::Select(_)
                            | Command::RenameServer(_)
                            | Command::ActivateServer(_)
                            | Command::RemoveServer(_)
                            | Command::Edit { .. }
                            | Command::Playlists(_)
                            | Command::Report(_)
                    )
                {
                    continue;
                }
                if let Err(error) = runtime.block_on(worker.run(&job)) {
                    worker.emit(job.generation, Reply::Error(error));
                }
            }
        })
        .expect("unable to start the Jellyfin backend");
    tx
}

struct Worker {
    dirs: AppDirs,
    events: mpsc::Sender<Event>,
    generation: Arc<AtomicU64>,
    catalog_generation: Arc<AtomicU64>,
    catalog_request: Option<u64>,
    waker: Waker,
    source: Option<Source>,
    servers: Vec<Source>,
    token: Option<Token>,
    challenge: Option<Challenge>,
    pending_source: Option<Source>,
    pending_client: Option<Client>,
    client: Option<Client>,
}

#[derive(Serialize, Deserialize)]
struct SavedAccounts {
    servers: Vec<Source>,
    active_id: Option<String>,
}

impl Worker {
    fn current(&self, generation: u64) -> bool {
        self.catalog_request.map_or_else(
            || self.generation.load(Ordering::SeqCst) == generation,
            |request| self.catalog_generation.load(Ordering::SeqCst) == request,
        )
    }
    fn emit(&self, generation: u64, reply: Reply) {
        if self.current(generation) {
            let reply = match (self.catalog_request, reply) {
                (Some(request), Reply::Data { rows, cached }) => {
                    let Some(source) = self.source.clone() else {
                        return;
                    };
                    Reply::Catalog {
                        request,
                        source,
                        rows,
                        cached,
                    }
                }
                (Some(request), Reply::Error(error)) => Reply::CatalogError { request, error },
                (_, reply) => reply,
            };
            let _ = self.events.send(Event { generation, reply });
            self.waker.wake();
        }
    }
    fn account_file(&self) -> PathBuf {
        self.dirs.state.join("jellyfin-account.json")
    }
    fn revoked_file(&self) -> PathBuf {
        self.dirs.state.join("jellyfin-revoked")
    }
    fn persist_accounts(&self) -> Result<(), String> {
        let accounts = SavedAccounts {
            servers: self.servers.clone(),
            active_id: self.source.as_ref().map(|source| source.id.clone()),
        };
        fastframe_file::write(
            &self.account_file(),
            serde_json::to_vec(&accounts).map_err(|_| "Could not save Jellyfin accounts.")?,
        )
        .map_err(|_| "Could not save Jellyfin accounts.".to_owned())
    }

    async fn load_playlists(
        &self,
        source: &Source,
        proxy: &crate::settings::ProxyConfig,
    ) -> Result<Vec<crate::jellyfin::Item>, String> {
        if self.source.as_ref() != Some(source) {
            return Err("Jellyfin account changed.".into());
        }
        let token = self.token.as_ref().ok_or("Connect to Jellyfin again.")?;
        let client = Client::new(&source.server_url, &source.device_id, proxy)
            .map_err(|error| error.to_string())?;
        let mut items = Vec::new();
        loop {
            let page = client
                .query(
                    &source.user_id,
                    &crate::jellyfin::catalog::Query::Playlists,
                    items.len() as u32,
                    token,
                )
                .await
                .map_err(|error| error.to_string())?;
            let done = page.items.is_empty()
                || page.start_index.saturating_add(page.items.len() as u32)
                    >= page.total_record_count;
            items.extend(page.items);
            if items.len() > 10000 {
                return Err("The playlist picker exceeds its item limit.".into());
            }
            if done {
                return Ok(items);
            }
        }
    }

    async fn edit_playlist(
        &self,
        source: &Source,
        edit: &PlaylistEdit,
        proxy: &crate::settings::ProxyConfig,
    ) -> Result<Option<(String, String)>, String> {
        if self.source.as_ref() != Some(source) {
            return Err("Jellyfin account changed. Open the playlist again.".into());
        }
        let token = self
            .token
            .as_ref()
            .ok_or("Connect to Jellyfin again before editing playlists.")?;
        let client = Client::new(&source.server_url, &source.device_id, proxy)
            .map_err(|error| error.to_string())?;
        let result = match edit {
            PlaylistEdit::Create {
                name,
                public,
                items,
            } => {
                let id = client
                    .create_playlist_with_items(&source.user_id, name, *public, items, token)
                    .await
                    .map_err(|error| error.to_string())?;
                return Ok(Some((id, name.clone())));
            }
            PlaylistEdit::Rename { id, name } => client.rename_playlist(id, name, token).await,
            PlaylistEdit::Delete { id } => client.delete_playlist(id, token).await,
            PlaylistEdit::Add { id, items } => {
                client
                    .add_to_playlist(&source.user_id, id, items, token)
                    .await
            }
            PlaylistEdit::Remove { id, entries } => {
                client.remove_playlist_entries(id, entries, token).await
            }
            PlaylistEdit::Move { id, entry, index } => {
                client.move_playlist_entry(id, entry, *index, token).await
            }
        };
        result.map(|()| None).map_err(|error| error.to_string())
    }

    async fn send_report(
        &self,
        source: &Source,
        token: &Token,
        report: &crate::jellyfin::reporting::Report,
        proxy: &crate::settings::ProxyConfig,
    ) -> Result<(), crate::jellyfin::Error> {
        let client = Client::new(&source.server_url, &source.device_id, proxy)?;
        client
            .report_playback(
                &report.playback.item,
                report.playback.position_ms,
                !report.playback.playing,
                report.event,
                token,
            )
            .await
    }

    async fn run(&mut self, job: &Job) -> Result<(), String> {
        let generation = job.generation;
        match &job.command {
            Command::Report(report) => {
                let source = &report.playback.source;
                if !self.servers.iter().any(|saved| saved.id == source.id) {
                    return Ok(());
                }
                let result = if self.source.as_ref() == Some(source) {
                    if let Some(token) = &self.token {
                        self.send_report(source, token, report, &job.proxy).await
                    } else {
                        return Ok(());
                    }
                } else {
                    if let Some(token) = credentials(source.clone(), CredentialOp::Read)? {
                        self.send_report(source, &token, report, &job.proxy).await
                    } else {
                        return Ok(());
                    }
                };
                let error = result.err().map(|error| {
                    format!("Jellyfin listening history could not be updated: {error}")
                });
                let _ = self.events.send(Event {
                    generation,
                    reply: Reply::ReportError {
                        source: source.clone(),
                        error,
                    },
                });
                self.waker.wake();
            }

            Command::Playlists(source) => {
                let result = self.load_playlists(source, &job.proxy).await;
                let _ = self.events.send(Event {
                    generation,
                    reply: Reply::Playlists {
                        source: source.clone(),
                        result,
                    },
                });
                self.waker.wake();
            }

            Command::Edit {
                source,
                request,
                edit,
            } => {
                let result = self.edit_playlist(source, edit, &job.proxy).await;
                let _ = self.events.send(Event {
                    generation,
                    reply: Reply::Edited {
                        source: source.clone(),
                        request: *request,
                        result,
                    },
                });
                self.waker.wake();
            }

            Command::Select(provider) => {
                fastframe_file::write(
                    &self.dirs.state.join("browsing-provider.json"),
                    serde_json::to_vec(provider)
                        .map_err(|_| "Could not save provider selection.")?,
                )
                .map_err(|_| "Could not save provider selection.")?;
            }
            Command::RenameServer(source) => {
                if let Some(saved) = self.servers.iter_mut().find(|saved| saved.id == source.id) {
                    *saved = source.clone();
                    if self
                        .source
                        .as_ref()
                        .is_some_and(|active| active.id == source.id)
                    {
                        self.source = Some(source.clone());
                    }
                    self.persist_accounts()?;
                    self.emit(generation, Reply::Renamed(source.clone()));
                }
            }
            Command::Restore => {
                let provider = std::fs::read(self.dirs.state.join("browsing-provider.json"))
                    .ok()
                    .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                    .unwrap_or_default();
                let bytes = std::fs::read(self.account_file()).ok();
                let legacy: Option<Source> = bytes
                    .as_deref()
                    .and_then(|b| serde_json::from_slice(b).ok());
                let registry: Option<SavedAccounts> = bytes
                    .as_deref()
                    .and_then(|b| serde_json::from_slice(b).ok());
                let (mut servers, active_id) = registry
                    .map(|r| (r.servers, r.active_id))
                    .unwrap_or_else(|| (legacy.into_iter().collect(), None));
                servers.retain(valid_source);
                for source in &mut servers {
                    if source.nickname.trim().is_empty() {
                        source.nickname = source
                            .server_url
                            .trim_end_matches('/')
                            .rsplit('/')
                            .next()
                            .unwrap_or("Jellyfin")
                            .to_owned();
                        if source.nickname.is_empty() {
                            source.nickname = "Jellyfin".into();
                        }
                    }
                }
                let legacy_revoked = std::fs::read_to_string(self.revoked_file()).ok();
                servers.retain(|source| legacy_revoked.as_deref() != Some(&source.id));
                self.servers = servers;
                self.source = active_id
                    .and_then(|id| self.servers.iter().find(|source| source.id == id).cloned())
                    .or_else(|| self.servers.first().cloned());
                self.emit(
                    generation,
                    Reply::Restored {
                        source: self.source.clone(),
                        servers: self.servers.clone(),
                        provider,
                    },
                );
                if let Some(source) = &self.source {
                    self.client = Some(
                        Client::new(&source.server_url, &source.device_id, &job.proxy)
                            .map_err(|e| e.to_string())?,
                    );
                    self.token = credentials(source.clone(), CredentialOp::Read)?;
                    if self.token.is_none() {
                        return Err(
                            "Saved sign-in is unavailable. Sign out, then connect again.".into(),
                        );
                    }
                }
            }
            Command::ActivateServer(id) => {
                let source = self
                    .servers
                    .iter()
                    .find(|source| source.id == *id)
                    .cloned()
                    .ok_or("Saved Jellyfin server was not found.")?;
                let token = credentials(source.clone(), CredentialOp::Read)?
                    .ok_or("Saved sign-in is unavailable. Remove this server and connect again.")?;
                let client = Client::new(&source.server_url, &source.device_id, &job.proxy)
                    .map_err(|e| e.to_string())?;
                self.source = Some(source.clone());
                self.token = Some(token);
                self.client = Some(client);
                self.persist_accounts()?;
                self.emit(generation, Reply::Activated(source));
            }
            Command::RemoveServer(id) => {
                if let Some(index) = self.servers.iter().position(|source| source.id == *id) {
                    let source = self.servers.remove(index);
                    if self
                        .source
                        .as_ref()
                        .is_some_and(|active| active.id == source.id)
                    {
                        self.source = None;
                        self.token = None;
                        self.client = None;
                    }
                    self.persist_accounts()?;
                    let _ =
                        std::fs::remove_dir_all(self.dirs.cache.join("jellyfin").join(&source.id));
                    credentials(source, CredentialOp::Delete)?;
                    self.emit(
                        generation,
                        Reply::ServersChanged {
                            servers: self.servers.clone(),
                            active: self.source.clone(),
                        },
                    );
                }
            }
            Command::Begin(server) => {
                let id = format!("{:032x}", rand::random::<u128>());
                let client = Client::new(server, &id, &job.proxy).map_err(|e| e.to_string())?;
                let info = client.server_info().await.map_err(|e| e.to_string())?;
                let challenge = client.initiate().await.map_err(|e| e.to_string())?;
                if !self.current(generation) {
                    return Ok(());
                }
                self.pending_source = Some(Source {
                    id: id.clone(),
                    nickname: String::new(),
                    server_url: client.server_url().into(),
                    server_id: info.id,
                    user_id: String::new(),
                    device_id: id,
                });
                self.emit(generation, Reply::Challenge(challenge.code.clone()));
                self.challenge = Some(challenge);
                self.pending_client = Some(client);
            }
            Command::Poll => {
                let client = self
                    .pending_client
                    .as_ref()
                    .ok_or("Connect to a server first.")?;
                let challenge = self
                    .challenge
                    .as_ref()
                    .ok_or("Quick Connect is no longer active.")?;
                let account = match client.finish(challenge).await {
                    Ok(account) => account,
                    Err(crate::jellyfin::Error::Pending) => {
                        self.emit(generation, Reply::Pending);
                        return Ok(());
                    }
                    Err(error) => return Err(error.to_string()),
                };
                if !self.current(generation) {
                    return Ok(());
                }
                let Account {
                    user_id,
                    server_id,
                    token,
                } = account;
                let mut source = self
                    .pending_source
                    .clone()
                    .ok_or("Connect to a server first.")?;
                if source.server_id != server_id {
                    return Err("Server identity changed during sign-in. Connect again.".into());
                }
                source.user_id = user_id;
                if source.nickname.trim().is_empty() {
                    source.nickname = source
                        .server_url
                        .trim_end_matches('/')
                        .rsplit('/')
                        .next()
                        .unwrap_or("Jellyfin")
                        .to_owned();
                    if source.nickname.is_empty() {
                        source.nickname = "Jellyfin".into();
                    }
                }
                credentials(source.clone(), CredentialOp::Write(token.0.clone()))?;
                if !self.current(generation) {
                    let _ = credentials(source, CredentialOp::Delete);
                    return Ok(());
                }
                self.servers.retain(|saved| saved.id != source.id);
                self.servers.push(source.clone());
                let previous = self.source.clone();
                self.source = Some(source.clone());
                if let Err(error) = self.persist_accounts() {
                    self.source = previous;
                    self.servers.retain(|saved| saved.id != source.id);
                    let _ = credentials(source, CredentialOp::Delete);
                    return Err(error);
                }
                if !self.current(generation) {
                    self.servers.retain(|saved| saved.id != source.id);
                    let _ = self.persist_accounts();
                    let _ = credentials(source, CredentialOp::Delete);
                    return Ok(());
                }
                self.source = Some(source.clone());
                self.token = Some(token);
                self.challenge = None;
                self.pending_source = None;
                self.pending_client = None;
                self.client = Some(
                    Client::new(&source.server_url, &source.device_id, &job.proxy)
                        .map_err(|e| e.to_string())?,
                );
                self.emit(generation, Reply::Connected(source));
            }
            Command::Browse { view, start } => {
                let source = self.source.as_ref().ok_or("Connect to a server first.")?;
                let path = cache_file(&self.dirs, source, view, *start);
                let identity = format!("{}:{}:{}", source.id, source.server_id, source.user_id);
                if let Some(rows) = crate::account_cache::read::<Rows>(&path, &identity).await {
                    self.emit(generation, Reply::Data { rows, cached: true });
                }
                let token = self
                    .token
                    .as_ref()
                    .ok_or("Saved sign-in is unavailable. Sign out, then connect again.")?;
                // Every operation uses the latest proxy policy, including a
                // change made while browsing the other provider.
                let client = Client::new(&source.server_url, &source.device_id, &job.proxy)
                    .map_err(|e| e.to_string())?;
                let rows = match view {
                    View::Settings => return Ok(()),
                    View::Search {
                        library,
                        term,
                        kind,
                    } => {
                        let mut sections = Vec::new();
                        for category in [
                            crate::jellyfin::catalog::Kind::Song,
                            crate::jellyfin::catalog::Kind::Artist,
                            crate::jellyfin::catalog::Kind::Album,
                            crate::jellyfin::catalog::Kind::Playlist,
                        ] {
                            if kind.is_some_and(|kind| kind != category) {
                                continue;
                            }
                            if !self.current(generation) {
                                return Ok(());
                            }
                            let page = client
                                .query(
                                    &source.user_id,
                                    &crate::jellyfin::catalog::Query::Search {
                                        library: library.clone(),
                                        term: term.clone(),
                                        kind: category,
                                    },
                                    *start,
                                    token,
                                )
                                .await
                                .map_err(|e| e.to_string())?;
                            sections.push((category, page));
                        }
                        Rows::Search(sections)
                    }
                    View::Catalog { query, .. } => {
                        let page = client
                            .query(&source.user_id, query, *start, token)
                            .await
                            .map_err(|e| e.to_string())?;
                        Rows::Items {
                            items: page.items,
                            start: page.start_index,
                            total: page.total_record_count,
                        }
                    }
                    View::Libraries => Rows::Libraries(
                        client
                            .music_libraries(&source.user_id, token)
                            .await
                            .map_err(|e| e.to_string())?,
                    ),
                    View::Albums { library_id, .. } => {
                        let page = client
                            .albums(&source.user_id, library_id, *start, token)
                            .await
                            .map_err(|e| e.to_string())?;
                        Rows::Items {
                            items: page.items,
                            start: page.start_index,
                            total: page.total_record_count,
                        }
                    }
                    View::Tracks { album_id, .. } => {
                        let page = client
                            .album_tracks(&source.user_id, album_id, *start, token)
                            .await
                            .map_err(|e| e.to_string())?;
                        Rows::Items {
                            items: page.items,
                            start: page.start_index,
                            total: page.total_record_count,
                        }
                    }
                };
                if !self.current(generation) {
                    return Ok(());
                }
                // Cache failure must not hide a successful live response.
                let _ = crate::account_cache::write(
                    &path,
                    &crate::account_cache::Cache::new(identity, rows.clone()),
                )
                .await;
                self.emit(
                    generation,
                    Reply::Data {
                        rows,
                        cached: false,
                    },
                );
            }
            Command::Cancel => {
                self.challenge = None;
                self.pending_source = None;
                self.pending_client = None;
            }
            Command::SignOut => {
                if let Some(source) = self.source.clone() {
                    self.source = None;
                    self.token = None;
                    self.client = None;
                    self.servers.retain(|saved| saved.id != source.id);
                    self.persist_accounts()?;
                    let _ =
                        std::fs::remove_dir_all(self.dirs.cache.join("jellyfin").join(&source.id));
                    credentials(source, CredentialOp::Delete)?;
                    if let Some(next) = self.servers.first().cloned() {
                        let token = credentials(next.clone(), CredentialOp::Read)?
                            .ok_or("Saved sign-in is unavailable.")?;
                        self.client = Some(
                            Client::new(&next.server_url, &next.device_id, &job.proxy)
                                .map_err(|e| e.to_string())?,
                        );
                        self.token = Some(token);
                        self.source = Some(next);
                        self.persist_accounts()?;
                    }
                }
                self.emit(
                    generation,
                    Reply::Restored {
                        source: self.source.clone(),
                        servers: self.servers.clone(),
                        provider: Provider::Jellyfin,
                    },
                );
            }
        }
        Ok(())
    }
}

pub fn cache_file(dirs: &AppDirs, source: &Source, view: &View, start: u32) -> PathBuf {
    use sha2::{Digest, Sha256};
    let key = serde_json::to_vec(&(view, start)).expect("serializable Jellyfin view");
    let name = format!("{:x}.json", Sha256::digest(key));
    dirs.cache.join("jellyfin").join(&source.id).join(name)
}

enum CredentialOp {
    Read,
    Write(String),
    Delete,
}

fn credentials(source: Source, operation: CredentialOp) -> Result<Option<Token>, String> {
    // OS calls can block on an inaccessible store. Keep them off both the UI
    // and network runtime, with a bounded wait for a useful error.
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = native_credentials(&source, operation);
        let _ = tx.send(result);
    });
    rx.recv_timeout(std::time::Duration::from_secs(20))
        .map_err(|_| "The credential store did not respond. Unlock it and try again.".to_owned())?
}

fn native_credentials(source: &Source, operation: CredentialOp) -> Result<Option<Token>, String> {
    #[cfg(target_os = "linux")]
    let store = zbus_secret_service_keyring_store::Store::new();
    #[cfg(target_os = "macos")]
    let store = apple_native_keyring_store::keychain::Store::new();
    #[cfg(windows)]
    let store = windows_native_keyring_store::Store::new();
    let entry = store
        .and_then(|store| store.build(SERVICE, &format!("{}:{}", source.id, source.user_id), None))
        .map_err(|_| "Could not open the credential store. Unlock it and try again.".to_owned())?;
    match operation {
        CredentialOp::Read => match entry.get_secret() {
            Ok(bytes) => String::from_utf8(bytes)
                .ok()
                .and_then(|value| Token::new(value).ok())
                .ok_or_else(|| "Saved Jellyfin sign-in is invalid.".to_owned())
                .map(Some),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(_) => Err(
                "Could not read Jellyfin sign-in. Unlock the credential store and retry.".into(),
            ),
        },
        CredentialOp::Write(value) => entry
            .set_secret(value.as_bytes())
            .map(|()| None)
            .map_err(|_| "Could not save Jellyfin sign-in in the credential store.".into()),
        CredentialOp::Delete => match entry.delete_credential() {
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(_) => Err(
                "Sign-in was revoked locally, but the credential store could not delete its token."
                    .into(),
            ),
        },
    }
}

fn valid_source(source: &Source) -> bool {
    source.id.len() == 32
        && source.id.bytes().all(|byte| byte.is_ascii_hexdigit())
        && crate::jellyfin::validate_id(&source.user_id).is_ok()
        && crate::jellyfin::validate_id(&source.server_id).is_ok()
}

/// Non-secret saved identity, checked before restoring provider state.
pub(crate) fn saved_playback_source(dirs: &AppDirs) -> Result<Source, String> {
    let bytes = std::fs::read(dirs.state.join("jellyfin-account.json"))
        .map_err(|_| "Sign into Jellyfin in the app first.")?;
    let source: Source = serde_json::from_slice::<SavedAccounts>(&bytes)
        .ok()
        .and_then(|accounts| {
            accounts
                .active_id
                .and_then(|id| {
                    accounts
                        .servers
                        .iter()
                        .find(|source| source.id == id)
                        .cloned()
                })
                .or_else(|| accounts.servers.into_iter().next())
        })
        .or_else(|| serde_json::from_slice(&bytes).ok())
        .filter(valid_source)
        .ok_or("Sign into Jellyfin in the app first.")?;
    if std::fs::read_to_string(dirs.state.join("jellyfin-revoked"))
        .ok()
        .as_deref()
        == Some(&source.id)
    {
        return Err("Saved Jellyfin sign-in was revoked.".into());
    }
    Ok(source)
}

/// Decoder-worker credential lookup, bound to the account selected at click
/// time. No protected-store call or authenticated request reaches the UI.
pub(crate) fn saved_playback_session(
    dirs: &AppDirs,
    expected: &Source,
    proxy: &crate::settings::ProxyConfig,
) -> Result<(Client, Token), String> {
    let bytes = std::fs::read(dirs.state.join("jellyfin-account.json"))
        .map_err(|_| "Sign into Jellyfin in the app first.")?;
    let saved = serde_json::from_slice::<SavedAccounts>(&bytes)
        .ok()
        .map(|accounts| accounts.servers)
        .unwrap_or_else(|| {
            serde_json::from_slice::<Source>(&bytes)
                .ok()
                .into_iter()
                .collect()
        });
    let source = saved
        .into_iter()
        .find(|source| source.id == expected.id && source == expected)
        .filter(valid_source)
        .ok_or("The Jellyfin account changed or was removed. Choose the track again.")?;
    if std::fs::read_to_string(dirs.state.join("jellyfin-revoked"))
        .ok()
        .as_deref()
        == Some(&source.id)
    {
        return Err("Saved Jellyfin sign-in was revoked.".into());
    }
    let token = credentials(source.clone(), CredentialOp::Read)?
        .ok_or("Saved Jellyfin sign-in is unavailable.")?;
    let client = Client::new(&source.server_url, &source.device_id, proxy)
        .map_err(|error| error.to_string())?;
    Ok((client, token))
}

/// Explicit development diagnostic, separate from the application transport.
pub fn probe_saved_playback(
    dirs: &AppDirs,
    proxy: &crate::settings::ProxyConfig,
    library_name: Option<&str>,
    audible: bool,
    container: Option<&str>,
) -> Result<crate::jellyfin::spike::Report, String> {
    let source = saved_playback_source(dirs)?;
    if std::fs::read_to_string(dirs.state.join("jellyfin-revoked"))
        .ok()
        .as_deref()
        == Some(&source.id)
    {
        return Err("Saved Jellyfin sign-in was revoked.".into());
    }
    let token = credentials(source.clone(), CredentialOp::Read)?
        .ok_or("Saved Jellyfin sign-in is unavailable.")?;
    let client = Client::new(&source.server_url, &source.device_id, proxy)
        .map_err(|error| error.to_string())?;
    crate::jellyfin::spike::run_for_user(
        client,
        token,
        library_name,
        proxy,
        audible,
        &source.user_id,
        container,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> Source {
        Source {
            id: format!("{:032x}", rand::random::<u128>()),
            nickname: "Test server".into(),
            server_url: "https://music.example/jellyfin/".into(),
            server_id: "server1".into(),
            user_id: "user1".into(),
            device_id: "device1".into(),
        }
    }

    #[test]
    fn saved_account_registry_restores_the_selected_account_and_legacy_files() {
        let first = source();
        let mut second = source();
        second.id = "b".repeat(32);
        second.nickname = "Upstairs".into();
        let registry = SavedAccounts {
            servers: vec![first.clone(), second.clone()],
            active_id: Some(second.id.clone()),
        };
        let bytes = serde_json::to_vec(&registry).unwrap();
        let loaded: SavedAccounts = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(loaded.servers.len(), 2);
        assert_eq!(
            loaded
                .active_id
                .and_then(|id| loaded.servers.into_iter().find(|server| server.id == id)),
            Some(second)
        );
        let old_format = serde_json::to_vec(&first).unwrap();
        assert_eq!(
            serde_json::from_slice::<Source>(&old_format).unwrap(),
            first
        );
    }

    #[test]
    fn playback_rejects_replaced_and_revoked_accounts_before_credential_lookup() {
        let source = source();
        let root = std::env::current_dir()
            .unwrap()
            .join("target/jellyfin-account-tests")
            .join(&source.id);
        std::fs::create_dir_all(&root).unwrap();
        let dirs = AppDirs {
            config: root.clone(),
            state: root.clone(),
            cache: root.clone(),
        };
        std::fs::write(
            root.join("jellyfin-account.json"),
            serde_json::to_vec(&source).unwrap(),
        )
        .unwrap();
        let mut other = source.clone();
        other.user_id = "other-user".into();
        let error = saved_playback_session(&dirs, &other, &crate::settings::ProxyConfig::Off)
            .err()
            .unwrap();
        assert!(error.contains("account changed"));
        std::fs::write(root.join("jellyfin-revoked"), &source.id).unwrap();
        let error = saved_playback_session(&dirs, &source, &crate::settings::ProxyConfig::Off)
            .err()
            .unwrap();
        assert!(error.contains("revoked"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn page_cache_never_crosses_source_account_or_browsing_scope() {
        let root = std::env::current_dir()
            .unwrap()
            .join("target/jellyfin-cache-tests")
            .join(format!("{:032x}", rand::random::<u128>()));
        let dirs = AppDirs {
            config: root.clone(),
            state: root.clone(),
            cache: root.clone(),
        };
        let first = source();
        let second = source();
        let view = View::Albums {
            library_id: "music1".into(),
            name: "Music".into(),
        };
        let path = cache_file(&dirs, &first, &view, 0);
        assert_ne!(path, cache_file(&dirs, &second, &view, 0));
        assert_ne!(path, cache_file(&dirs, &first, &view, 100));
        let rows = Rows::Items {
            items: vec![],
            start: 0,
            total: 0,
        };
        crate::account_cache::write(
            &path,
            &crate::account_cache::Cache::new("source:server:user1".into(), rows),
        )
        .await
        .unwrap();
        assert!(
            crate::account_cache::read::<Rows>(&path, "source:server:user1")
                .await
                .is_some()
        );
        assert!(
            crate::account_cache::read::<Rows>(&path, "source:server:user2")
                .await
                .is_none()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn saved_source_rejects_path_traversal_before_any_cache_or_keychain_access() {
        let mut source = source();
        assert!(valid_source(&source));
        source.id = "../../other".into();
        assert!(!valid_source(&source));
    }

    #[test]
    #[ignore = "manual temporary-token round trip against the unlocked native store"]
    fn native_store_round_trip() {
        let source = source();
        credentials(
            source.clone(),
            CredentialOp::Write("temporary-test-token".into()),
        )
        .unwrap();
        let read = credentials(source.clone(), CredentialOp::Read);
        let removed = credentials(source.clone(), CredentialOp::Delete);
        assert_eq!(read.unwrap().unwrap().0, "temporary-test-token");
        removed.unwrap();
        assert!(credentials(source, CredentialOp::Read).unwrap().is_none());
    }
}
