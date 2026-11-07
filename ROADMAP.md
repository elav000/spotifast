# Jellyfin fork roadmap

This is the current working roadmap. The chronological implementation notes in
[the fork reference](docs/_reference/jellyfin-fork.md) describe earlier stages.
Accepted work and completed live validation are recorded in the
[completed milestones reference](docs/_reference/completed-milestones.md).

## Product and architecture decisions

- Feed existing Tumlo views and infrastructure through provider data and
  capabilities. Do not create parallel Jellyfin panels when an adapter will do.
- Keep shared page hierarchy, control placement and interaction patterns aligned
  across providers. Providers supply data and supported actions; they do not
  receive bespoke layouts merely because their capabilities differ. Put
  secondary provider actions in the shared overflow pattern and omit unsupported
  actions rather than rearranging the page around them.
- Keep Spotify and Jellyfin catalogs separate for now. Switching the catalog
  being browsed preserves the current playback provider and its queue; starting
  a song transfers output ownership. Switching active Jellyfin server/accounts
  still stops that server's transport and clears its queue. Queue actions must
  never be retargeted across a provider or account boundary.
- Use the term **Combined provider** for future catalog composition. This is a
  later feature, potentially including Plex, not the current separate-catalog UI.
- Entries retain provider/source/account/item identity. Do not invent Spotify
  identities for Jellyfin, or send Jellyfin entries to Spotify services.
- Share local playback policy where practical, while keeping service transports
  and service-specific capabilities in adapters. Spotify Connect remote queues
  remain Spotify-only.
- Preserve upstream behavior and make future upstream updates routine. See
  [the recorded sync baseline and procedure](docs/_reference/upstream-sync.md).
- Install validated Mac milestones in `/Applications/Tumlo.app`, refresh the
  development ZIP, and leave a running instance alone until the user relaunches.

## Current architecture state

Spotify and Jellyfin share `ManualQueue<T>` for ordered manual occurrences,
immutable row snapshots, move, front-consumption and batch-removal. Backward cursor history,
Next/Previous decisions, context play order and current-track re-anchoring are
provider-neutral primitives. `LocalPlaybackController<T>` now groups the local
context, manual queue and Previous history; Jellyfin uses it as the authoritative
state while retaining its saved JSON shape. Its main queue page, panel and
Winamp mini playlist accept provider track drags for insertion and manual-row
reordering, guarded by source identity and the queue snapshot the UI drew. The
empty Playing next section has its own drop slot; context rows remain outside
the target. Spotify Connect continues to own its remote queue and shuffle order.

Spotify local playback already reuses `ManualQueue<T>`, but it does not have a
second local transition controller to adopt. librespot's Connect state drives
Spotify context transitions and the reported queue, while `app.rs` keeps an
optimistic, reconciled view for the UI and queue edits. Moving those transitions
under `LocalPlaybackController<T>` would duplicate or obscure Connect ownership.
Keep Spotify's transitions in its adapter and treat the shared queue primitive
as the integration point for now. Revisit only if librespot exposes a distinct
local-only playback mode with its own context and transitions.

Jellyfin Recent now loads the active user's played audio across libraries,
sorted by server last-played date, while Previous remains the local controller's
bounded cursor history. The server API stores per-item played state rather than
an occurrence log, so Recent lists each song once. See the
[queue contract](docs/_reference/queue.md) and
[completed milestones](docs/_reference/completed-milestones.md).

The shared local controller milestone is complete for current provider needs:
provider execution adapters resolve and play entries, while exclusive output
ownership coordinates handoffs. Do not rewrite Spotify Connect's remote service
queue as part of this extraction.

The sign-in screen now offers Spotify or Jellyfin. The profile menu's
**Provider** submenu switches the browsed catalog and expands Jellyfin into
saved servers/accounts when needed. Settings has an **Additional providers**
section for Jellyfin nicknames, activation, removal and adding a connection
without signing out of the current one. See the
[completed milestone notes](docs/_reference/completed-milestones.md).

The central shell now receives its route, stable scroll key, overview/tint
behavior and global-search state from the active provider presentation. The top
bar and sidebar no longer inspect Jellyfin search state directly; provider
actions own query edits, debounce and submission while the shared search page
keeps its existing renderers.

The shared player bar now receives album, show and artist destinations from the
playback provider. Jellyfin keeps the playback source/account identity in those
actions and preserves linked artist IDs from its playback metadata; Spotify
retains its existing album, show and artist routes.

Browsing and playback provider selection are independent. The active song keeps
its player bar, queue, provider-specific links, device picker and transport
controls while another catalog is open. Starting playback from that catalog is
the handoff point and suspends the previous engine.

Active and queued entries carry a stable `ProviderStamp`. Spotify uses its
app-scoped identity; Jellyfin stamps the server and account without retaining a
credential or transport URL. Now-playing navigation, device/control selection,
queue ownership, every displayed queue occurrence and provider row actions read
that stamp rather than the catalog being browsed. This is the identity boundary
required before queues can contain entries from more than one provider.

Queue drag and edit capabilities now come from the playback-owned queue
presentation in both the main and Winamp views. The provider resolves drops
against its own source and snapshot, while the shared playlist dialogs receive
busy, error and deletion state from provider presentation instead of inspecting
Jellyfin state.

Multi-disc Spotify and Jellyfin albums use the same section rule and existing
track-row renderers. Provider disc numbers add headings only when an album has
more than one disc, retain server order and reset the displayed track number
within each disc. Single-disc albums keep their previous layout.

Album and artist overflow menus use one shared renderer and provider-owned
actions. Spotify retains its save, radio and service-link capabilities.
Jellyfin supplies play, shuffle, queue, favorite and server-link actions where
the server supports them; favorite changes are optimistic and account-scoped.
Artist hero controls share the same primary, saved-state and overflow hierarchy;
Jellyfin keeps Songs and Refresh as secondary overflow actions. Unsupported
service actions are omitted rather than rendered inertly.

The rebrand is complete for the app identity, artwork, README, About
attribution and fork links. `brand.toml` is the source for the display name,
macOS bundle ID and credential namespace; verified migration keeps existing
Spotifast Spotify and Jellyfin credentials. Legacy package, executable and
update asset names remain for compatibility. The fork has no GitHub releases,
and release publishing or release metadata migration is out of scope unless
the maintainer asks for one.

## Next implementation sequence

The reviewed Jellyfin sharp-edge, provider-orchestration, multi-disc album,
provider-stamping and provider-switching sequences are complete. No additional
implementation milestone is approved yet. The next planning target is the
Combined provider and mixed local queue described below; keep catalog
composition, playback handoff and shared cache policy as separate decisions.

## Later work

- Combined provider and mixed local queues after the shared controller is stable;
  Plex or other sources can follow through the same contracts. Every queued and
  active entry must retain the provider identity used for its links, devices and
  controls. Catalog composition, mixed playback and shared caches are independent
  decisions.

Incremental delivery within a partially received HTTP range is deferred. Revisit
only if timing data shows the Jellyfin server delivers useful body bytes before
the full range completes; otherwise changing the client cannot improve the
stall. Any implementation must preserve range/entity validation and bounded
memory use. The reader already retries transient failures twice; see the
[completed milestone notes](docs/_reference/completed-milestones.md).

## Where to continue in the CLI

Work on `main` in this repository. Read `AGENTS.md`, `CONTRIBUTING.md`, this file,
[queue rules](docs/_reference/queue.md), and the sync procedure before editing.
The older fork reference is historical context, not an instruction to redo work.

- `src/provider.rs`, `src/provider/queue.rs`: identities, shared presentations and
  source-bound queue capabilities.
- `src/jellyfin/browser.rs`, `src/jellyfin/transport.rs`: provider actions and the
  current Jellyfin playback/queue controller.
- `src/media/`: encoded transport, HTTP range cache and buffered audio work.
- `src/app.rs`, `src/player.rs`: existing application and Spotify playback policy.
- `src/ui/queue.rs`, `src/ui/winamp/playlist.rs`: existing main and mini queue views.
- `src/demo.rs`: deterministic provider/UI integration tests and capture fixtures.

NativeAmp in `~/Source/nativeamp` remains a useful reference for decoding/seeking.
It contains user changes; do not modify or reset it as part of Tumlo work.
Never log tokens, authorization headers or credential-bearing URLs. Use the
system credential store; no credentials belong in this roadmap or the repository.

The local desktop handoff is under
`~/Documents/Codex/2026-10-15/referenced-chatgpt-conversation-this-is-an/outputs/`:
`spotifast-jellyfin-port-plan.md` preserves chronological notes and the deferred
failure's track names; `jellyfin-long-mix-decode-failure.png` is the user screenshot.
These private artifacts and visual comparisons stay outside Git.
