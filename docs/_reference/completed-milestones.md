---
title: Completed Jellyfin fork milestones
description: Accepted provider work and live validation retained as project history.
---

This page records completed fork work for future maintenance and upstream
integration. Current priorities live in [`ROADMAP.md`](../../ROADMAP.md); the
history below is not a list of tasks to repeat.

## Provider integration

- Protected Jellyfin sign-in and restoration for one server/account, separate
  browsing selection, account-scoped catalog/page caches and artwork caching.
- Sign-in offers Spotify and Jellyfin as provider choices. The main provider
  dropdown was removed; Settings switches providers and manages multiple saved
  Jellyfin servers/accounts, including nicknames, activation, removal and
  adding a connection without signing out of the active account.
- Server nicknames are presentation-only. Playback restoration, queue actions,
  in-flight browser replies and listening-history reporting use stable server
  and account identity, so renaming a server during playback does not interrupt
  the session or report a false stop/start.
- Existing left rail, search, album/artist/playlist pages, grids, track rows and
  playlist dialogs fed by provider adapters. Provider and Jellyfin collection
  selection live in menus, preserving the existing rail layout.
- Jellyfin search defaults to all music collections and offers an explicit
  collection picker for narrowing songs, artists and albums. Search scope is
  independent of the Home collection selection and follows artist navigation;
  playlists remain account-scoped because Jellyfin does not place them under a
  music collection.
- The active provider now supplies the central shell's page route, stable scroll
  identity, overview/tint behavior and global-search state. The shared top bar,
  sidebar and debounce path no longer read Jellyfin search fields or browser
  routes directly, while Spotify and Jellyfin retain independent queries.
- The playback provider now supplies the shared player bar's album, show and
  artist destinations. Jellyfin keeps source/account identity in its actions
  and linked artist IDs in now-playing metadata, while Spotify retains its
  existing destinations.
- Queue drag/edit policy now belongs to the playback-owned presentation used by
  both main and Winamp queue views. Providers validate their own source and
  snapshot and return the resulting action. Shared playlist dialogs likewise
  receive busy, error and deletion state through provider presentation rather
  than inspecting Jellyfin state.
- Browsing provider selection no longer stops playback. The active playback
  provider continues to own the player bar, queue, links, device picker and
  controls until a song from another provider takes output ownership.
- Multi-disc Spotify and Jellyfin albums render through their existing shared
  page and row paths with provider disc numbers. Disc headings appear only when
  more than one disc is present, preserve provider order and show each disc's
  own track numbers; single-disc albums are unchanged.
- Jellyfin Home retains the music-library cards for selection and adds shared
  card shelves for Recently Added Music, Recently Played and Frequently Played.
  These account-scoped shelves use server ordering; Spotify Home is unchanged.
- Jellyfin playback through the shared transport controls, exclusive local
  output ownership and common encoded-media/cache infrastructure. The existing
  Spotify transport and its backend cache remain separate; there are no offline
  downloads.
- Bounded read-ahead/PCM buffering, whole-response network deadlines, visible
  errors and responsive controls. The user confirmed continuous playback after
  the buffering correction. Arbitrary future server stalls are not ruled out.
- HTTP audio ranges retry transient connection/body failures and temporary
  server errors at most twice, with bounded backoff. Each attempt starts at the
  range boundary and must validate the full range and entity before the decoder
  sees bytes. Integrity, authorization and entity-change errors are not retried;
  incremental delivery of incomplete ranges is deferred until timing evidence
  shows the server delivers useful bytes before a full range completes.
- Removed the bespoke Jellyfin queue panel. Main queue page/panel use the
  existing UI with `QueuePresentation`, preserving duplicate manual
  occurrences, manual priority and context continuation.
- Fixed the Winamp mini player closing when Spotify is signed out. Its existing
  PL pane uses Jellyfin queue data and provider actions for play, selected
  manual removal, clear/save, search, library/playlist loading and available
  album links.
- Atomic manual batch removal rejects stale queue snapshots and other
  accounts. Automated full-to-mini-to-full tests cover order, duplicates,
  selection, source ownership and retained playback state. Spotify queue save
  stays Spotify-owned even while browsing Jellyfin.
- Live Jellyfin queue acceptance passed: repeated manual entries matched in
  main and mini views; Next, playing a selected entry, selected removal, clear,
  pause, resume and seek behaved as expected. With a multi-track album playing,
  the manual entry appeared ahead of album context; Next and clear preserved
  the expected context.
- Live Jellyfin playlist load/save passed: loaded and played an existing
  playlist, saved the current queue as a private playlist and verified its
  current/upcoming tracks, then deleted the temporary saved copy. The original
  playlist remained.
- Jellyfin queue Recent now reads the active user's played audio across
  libraries, sorted by server last-played date and paginated. Jellyfin exposes
  per-item played state rather than a play-occurrence log, so each song appears
  once. The currently playing item is shown at the top; Previous remains the
  local controller's bounded cursor history.
- Catalog reads run on two cancellable lanes, one for the visible page and one
  for the sidebar library. A newer read replaces only the stale read in its
  lane, while sign-out, connection and server-account changes cancel both.
  Ordered account mutations, playlist work and playback reports remain on the
  account worker, so a slow multi-category search cannot delay them.
- Successful playlist edits invalidate the account's cached catalog pages
  before the confirmation refresh. Remove and Move keep their optimistic rows
  visible while that refresh ignores any pre-edit cached reply and waits for
  live Jellyfin data; failed edits still restore the previous occurrences.
- Soloist presentation and connection retries follow the playback provider.
  Jellyfin does not show Soloist setup or failure banners, block its playback
  controls, or wake a failed Soloist process. Browsing Spotify does not wake or
  take over playback; starting a Spotify song performs that handoff and wakes
  the configured player when needed.
- Switching the active Jellyfin server/account now follows that same boundary.
  Tumlo reports the outgoing session to its original account, then stops and
  clears its transport and queue before the next account becomes active. A
  successful new Quick Connect is guarded the same way, so an old item ID is
  never resolved or reported through the new account.

## Upstream integration

- Integrated 42 upstream commits through
  `ee8900d54693a807d657154e6b16ea572e963791`, including upstream 0.13.3.
  Preserved provider adapters and adapted the new fullscreen queue column. The
  combined lockfile's Nix vendor hash is refreshed.

## MP3 startup recovery

Reproduced on “Push It (March 2014 Mix)” (48 kHz stereo, 320 kbps MP3,
1:06:49). Symphonia rejects its first packet at timestamp zero with
`mpa: invalid main_data offset`. The current entity was checked online and its
exact-length cached opening range passed its cache checksum. FFmpeg decoded
that range but warned `invalid new backstep -1`; the Jellyfin web client also
plays the track. The failure was in handling this MP3's opening frame/reservoir,
not duration or a reported incomplete HTTP range. The earlier screenshot's
open album was not the current song; the recorded failing item was Push It.
An offline probe recovered at packet 27. The decoder now skips up to 64
undecodable leading MP3 packets (about 1.5 to 1.7 seconds at common sample
rates), with a regression test for a damaged leader. The user confirmed live
playback works with commit `08aa3f3`.

The original screenshot and track names are retained in the private workspace
plan. Private investigation artifacts and visual comparisons stay outside Git.
