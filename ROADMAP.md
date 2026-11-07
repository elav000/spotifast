# Jellyfin fork roadmap

This is the current working roadmap. The chronological implementation notes in
[the fork reference](docs/_reference/jellyfin-fork.md) describe earlier stages.
Accepted work and completed live validation are recorded in the
[completed milestones reference](docs/_reference/completed-milestones.md).

## Product and architecture decisions

- Feed existing Tumlo views and infrastructure through provider data and
  capabilities. Do not create parallel Jellyfin panels when an adapter will do.
- Keep Spotify and Jellyfin catalogs separate for now. Switching providers or
  active Jellyfin server/accounts stops the outgoing transport and clears its
  queue; queue actions must never be retargeted across that boundary.
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

The sign-in screen now offers Spotify or Jellyfin. The main source/account
dropdown is removed; Settings switches providers and manages saved Jellyfin
servers and accounts, including nicknames, activation, removal and adding a
connection without signing out of the current one. See the
[completed milestone notes](docs/_reference/completed-milestones.md).

The rebrand is complete for the app identity, artwork, README, About
attribution and fork links. `brand.toml` is the source for the display name,
macOS bundle ID and credential namespace; verified migration keeps existing
Spotifast Spotify and Jellyfin credentials. Legacy package, executable and
update asset names remain for compatibility. The fork has no GitHub releases,
and release publishing or release metadata migration is out of scope unless
the maintainer asks for one.

## Next implementation sequence

The reviewed Jellyfin sharp-edge sequence is complete. Continue with bounded
items under Later work as they become the next approved milestone.

## Later work

- **Finish provider-owned page orchestration.** The shared cards, track rows,
  detail heroes, controls, queue presentation and device capability are in
  place, but the application shell still branches on Jellyfin view state.
  Move central-page routing, global search state and playback-context
  navigation behind provider presentation/actions. Then make queue drag/edit
  capabilities and playlist dialogs explicitly provider-owned instead of
  checking Jellyfin source state. Keep provider setup and account management
  service-specific. Deliver these extractions through bounded user-facing
  fixes, not a parallel UI or a broad rewrite.

- **Show multi-disc albums as separate disc sections.** Use provider disc numbers
  to label and group tracks in the shared album page, while preserving their
  order and keeping the layout useful for single-disc albums. Jellyfin can use
  this where its library supplies disc numbers.

- Combined provider and mixed local queues after the shared controller is stable;
  Plex or other sources can follow through the same contracts. Catalog composition,
  mixed playback and shared caches are independent decisions.

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
