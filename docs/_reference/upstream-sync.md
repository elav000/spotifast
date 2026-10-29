---
title: Fork upstream sync
description: Keep the Jellyfin fork current with upstream while preserving clear GitHub ahead/behind counts.
---

The Jellyfin fork's `main` is based on upstream
`crmne/spotifast:main` at `b708e0a6bf5e0b25aa7d270716e89d4671739f61`.
Fork-only commits are rebased on that upstream ancestry, keeping GitHub's
ahead/behind count meaningful while preserving a linear history. The recovery
ref for the 2026-10-28 sync is
`recovery/pre-upstream-2026-10-28-837c52d`. Earlier recovery refs are
`recovery/pre-upstream-2026-10-25-e9ec4f4`,
`recovery/pre-upstream-2026-10-17-b56cd28`, and
`recovery/pre-upstream-rebase-2026-10-18-b977bb4`.

For each future update, fetch upstream and rebase the fork-only commits onto its
new `main`. Do not merge upstream or push to it. This rewrites published commits
on `elav000/spotifast`; do it only with explicit maintainer approval, create a
recovery ref first, and push to the fork using an exact `--force-with-lease`
guard. Aim to do this integration about once a week while fork work is active,
then let CI finish as the final sync check. Update this page to the new upstream
baseline after the rebase. Keep the fork's commits linear and verify the
ahead/behind count against upstream.

Resolve overlaps by preserving shared provider presentations and source-bound
actions. In particular, upstream queue surfaces must consume `playback_queue()`;
Spotify occurrence-aware rows must retain their position metadata, while
provider rows retain their explicit current/playing/loading state. Keep shared
encoded-media caching, output ownership and the fork update channel intact.

Run the contribution checks, provider queue/mini-player regressions and visual
smoke tests. The Nix package reads the combined lockfile directly, so dependency
updates need no separate vendor hash. Verify `nix build .#default` locally or in
CI. Install a verified Mac milestone without interrupting a running app.
