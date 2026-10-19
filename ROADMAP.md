# Jellyfin fork roadmap

This is the current working roadmap. The chronological implementation notes in
[the fork reference](docs/_reference/jellyfin-fork.md) describe earlier stages.
Accepted work and completed live validation are recorded in the
[completed milestones reference](docs/_reference/completed-milestones.md).

## Product and architecture decisions

- Feed existing Spotifast views and infrastructure through provider data and
  capabilities. Do not create parallel Jellyfin panels when an adapter will do.
- Keep Spotify and Jellyfin catalogs separate for now. Switching the provider
  being browsed must not change playback or retarget queue actions.
- Use the term **Combined provider** for future catalog composition. This is a
  later feature, potentially including Plex, not the current separate-catalog UI.
- Entries retain provider/source/account/item identity. Do not invent Spotify
  identities for Jellyfin, or send Jellyfin entries to Spotify services.
- Share local playback policy where practical, while keeping service transports
  and service-specific capabilities in adapters. Spotify Connect remote queues
  remain Spotify-only.
- Preserve upstream behavior and make future upstream updates routine. See
  [the recorded sync baseline and procedure](docs/_reference/upstream-sync.md).
- Install validated Mac milestones in `/Applications/Spotifast.app`, refresh the
  development ZIP, and leave a running instance alone until the user relaunches.

## Next architecture milestone

Continue extracting the shared local playback controller, reusing existing
provider rules and keeping execution in provider adapters. Spotify and Jellyfin
share `ManualQueue<T>` for ordered manual occurrences, immutable row snapshots,
move, front-consumption and batch-removal. Backward cursor history,
Next/Previous decisions, context play order and current-track re-anchoring are
provider-neutral primitives. `LocalPlaybackController<T>` now groups the local
context, manual queue and Previous history; Jellyfin uses it as the authoritative
state while retaining its saved JSON shape. Its main queue page and panel now
accept provider track drags for insertion and manual-row reordering, guarded by
source identity and the queue snapshot the UI drew. The empty Playing next section
has its own drop slot; context rows remain outside the target. Spotify Connect
continues to own its remote queue and shuffle order.

Next, assess whether any Spotify local-playback path can use the controller
without crossing into Connect service ownership. Keep regression coverage for
stale snapshots, account switches, repeated songs, seeking, playback errors and
full/mini window transitions. Provider execution adapters resolve and play
entries; exclusive output ownership coordinates handoffs. Do not rewrite Spotify
Connect's remote service queue as part of this extraction.

## Later work

- Revamp login, account/sign-in and server management: choose a provider during
  sign-in, then manage connected accounts and server(s) from Settings. This is
  future design work; no implementation is requested yet.
- Broader Jellyfin Recent history. The shared Recent tab currently shows bounded
  current-context playback history, not full server history.
- Shared Home layout with suitable Jellyfin sections; content remains undecided.
- Better recovery from transient range/network stalls, including bounded retries
  and incremental delivery within a not-yet-complete range where appropriate.
- Combined provider and mixed local queues after the shared controller is stable;
  Plex or other sources can follow through the same contracts. Catalog composition,
  mixed playback and shared caches are independent decisions.

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
It contains user changes; do not modify or reset it as part of Spotifast work.
Never log tokens, authorization headers or credential-bearing URLs. Use the
system credential store; no credentials belong in this roadmap or the repository.

The local desktop handoff is under
`~/Documents/Codex/2026-10-15/referenced-chatgpt-conversation-this-is-an/outputs/`:
`spotifast-jellyfin-port-plan.md` preserves chronological notes and the deferred
failure's track names; `jellyfin-long-mix-decode-failure.png` is the user screenshot.
These private artifacts and visual comparisons stay outside Git.
