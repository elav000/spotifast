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
- Live Jellyfin queue acceptance passed: repeated manual entries matched in main
  and mini views; Next, playing a selected entry, selected removal, clear, pause,
  resume and seek behaved as expected. With a multi-track album playing, the
  manual entry appeared ahead of album context; the maintainer confirmed Next
  and clear preserve the expected context.
- Live Jellyfin playlist load/save passed: loaded and played an existing playlist,
  saved the current queue as a private playlist and verified its current/upcoming
  tracks, then deleted the temporary saved copy. The original playlist remained.
- Integrated 42 upstream commits through `ee8900d54693a807d657154e6b16ea572e963791`,
  including upstream 0.13.3. Preserved provider adapters and adapted the new
  fullscreen queue column. The combined lockfile's Nix vendor hash is refreshed.

## Next session: bugs and live acceptance

1. **MP3 startup recovery is verified.** Reproduced on
   "Push It (March 2014 Mix)" (48 kHz stereo, 320 kbps MP3, 1:06:49). Symphonia
   rejects its first packet at timestamp zero with `mpa: invalid main_data
   offset`. The current entity was checked online and its exact-length cached
   opening range passed its cache checksum. FFmpeg decoded that range but warned
   `invalid new backstep -1`; the Jellyfin web client also plays the track. The
   failure is in handling this MP3's opening frame/reservoir, not duration or a
   reported incomplete HTTP range. The earlier screenshot's open album was not
   the current song; the recorded failing item was Push It. An offline probe
   recovered at packet 27. The decoder now skips up to 64 undecodable leading
   MP3 packets (about 1.5 to 1.7 seconds at common sample rates), with a
   regression test for a damaged leader. The user confirmed live playback works
   with commit `08aa3f3`. Keep the private workspace plan preserving the original
   screenshot and track names.

## Next architecture milestone

Continue extracting one common local playback/queue controller, not just a shared
renderer. The first shared foundation is `ManualQueue<T>`: Spotify and Jellyfin
now use the same ordered manual-occurrence storage, immutable row snapshots,
move, front-consumption and batch-removal rules. Their controllers remain
separate. Next, reuse the existing rules and tests for advancement, history,
shuffle, repeat and persistence. Provider execution adapters resolve and play
entries; exclusive output ownership coordinates handoffs. Do not rewrite
Spotify Connect's remote service queue as part of this extraction.

Then finish provider-neutral drag/drop payloads and Jellyfin queue insertion /
reordering. Move up/down menus already work; Jellyfin drag/drop is not complete.
Keep tests for stale snapshots, account switches, repeated songs, seeking,
playback errors and full/mini window transitions as the regression contract.

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
