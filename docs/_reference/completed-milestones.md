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
- Existing left rail, search, album/artist/playlist pages, grids, track rows and
  playlist dialogs fed by provider adapters. Provider and Jellyfin collection
  selection live in menus, preserving the existing rail layout.
- Jellyfin playback through the shared transport controls, exclusive local
  output ownership and common encoded-media/cache infrastructure. The existing
  Spotify transport and its backend cache remain separate; there are no offline
  downloads.
- Bounded read-ahead/PCM buffering, whole-response network deadlines, visible
  errors and responsive controls. The user confirmed continuous playback after
  the buffering correction. Arbitrary future server stalls are not ruled out.
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
