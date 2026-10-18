# Jellyfin fork roadmap

This is the current working roadmap. The chronological implementation notes in
[the fork reference](docs/_reference/jellyfin-fork.md) describe earlier stages;
completed milestones below supersede their old "next step" language.

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

## Completed milestones

- Protected Jellyfin sign-in and restoration for one server/account, separate
  browsing selection, account-scoped catalog/page caches and artwork caching.
- Existing left rail, search, album/artist/playlist pages, grids, track rows and
  playlist dialogs fed by provider adapters. Provider and Jellyfin collection
  selection live in menus, preserving the existing rail layout.
- Jellyfin playback through the shared transport controls, exclusive local output
  ownership and common encoded-media/cache infrastructure. Retain the existing
  Spotify transport and its backend cache; there are no offline downloads.
- Bounded read-ahead/PCM buffering, whole-response network deadlines, visible
  errors and responsive controls. The user confirmed continuous playback after
  the buffering correction. Arbitrary future server stalls are not ruled out.
- Removed the bespoke Jellyfin queue panel. Main queue page/panel use the existing
  UI with `QueuePresentation`, preserving duplicate manual occurrences, manual
  priority and context continuation.
- Fixed the Winamp mini player closing when Spotify is signed out. Its existing
  PL pane uses Jellyfin queue data and provider actions for play, selected manual
  removal, clear/save, search, library/playlist loading and available album links.
- Atomic manual batch removal rejects stale queue snapshots and other accounts.
  Automated full-to-mini-to-full tests cover order, duplicates, selection, source
  ownership and retained playback state. Spotify queue save stays Spotify-owned
  even while browsing Jellyfin.
- Integrated 42 upstream commits through `ee8900d54693a807d657154e6b16ea572e963791`,
  including upstream 0.13.3. Preserved provider adapters and adapted the new
  fullscreen queue column. The combined lockfile's Nix vendor hash is refreshed.

## Next session: bugs and live acceptance

1. **Investigate the deferred audio decoding failure.** A user screenshot shows
   "Audio decoding failed" at 0:00 for a long mix. The browsed album and Now playing
   are different mixes, so correlate actual item/attempt identity before deciding
   which file failed. Capture codec/container/media-source metadata, response
   completeness and the underlying decoder error. Check whether an obsolete
   attempt can leave an error on another song. Duration alone is not a diagnosis.
   The private workspace plan preserves track names and the screenshot. This was
   explicitly deferred during the upstream sync; no fix is claimed.
2. **Complete live main/mini queue acceptance.** The user says the mini playlist
   looks good; audible continuity and the complete workflow are not yet recorded
   as passed. Add a repeated song twice, compare order in both views, use Next,
   play a particular queued copy, remove selected/all manual entries, and switch
   back. Confirm current song, remaining duplicates/context, Pause/resume and seek.
3. **Verify playlist load/save against the live Jellyfin server.** Automated checks
   verify provider-bound actions; a real server mutation is still outstanding.
   Queue save currently uses the main view's private-playlist/deduplication policy.
4. **Show real mini-player media metadata.** The existing bitrate/sample-rate
   readout assumes Spotify's configured bitrate and 44 kHz. Feed actual Jellyfin
   media metadata through shared presentation rather than hard-coding a format.

## Next architecture milestone

Extract one common local playback/queue controller, not just a shared renderer.
The Spotify and Jellyfin controllers are still separate today. Reuse the existing
rules and tests for ordering, manual priority, duplicate occurrence identity,
advancement, history, shuffle, repeat and persistence. Provider execution adapters
resolve and play entries; exclusive output ownership coordinates handoffs.
Do not rewrite Spotify Connect's remote service queue as part of this extraction.

Then finish provider-neutral drag/drop payloads and Jellyfin queue insertion /
reordering. Move up/down menus already work; Jellyfin drag/drop is not complete.
Keep tests for stale snapshots, account switches, repeated songs, seeking,
playback errors and full/mini window transitions as the regression contract.

## Later work

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
