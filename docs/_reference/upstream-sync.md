---
title: Fork upstream sync
description: The upstream baseline integrated into the Jellyfin fork.
---

The Jellyfin fork includes upstream `crmne/spotifast` through
`ee8900d54693a807d657154e6b16ea572e963791` (42 commits after the original
`6a60c30` baseline). The integration preserves published fork commits and linear
history. The pre-sync recovery ref is
`recovery/pre-upstream-2026-10-17-b56cd28`.

Future updates should apply only the delta after this recorded upstream baseline,
using Git's three-way patch application, then update this page to the new baseline.
Do not rebase published Jellyfin work or force-push. A squash does not add an
upstream ancestry edge, so `git log HEAD..upstream/main` alone will also list
already integrated commits. Use the recorded baseline to identify new changes.

Resolve overlaps by preserving shared provider presentations and source-bound
actions. In particular, upstream queue surfaces must consume `playback_queue()`;
Spotify occurrence-aware rows must retain their position metadata, while
provider rows retain their explicit current/playing/loading state. Keep shared
encoded-media caching, output ownership and the fork update channel intact.

Run the contribution checks, provider queue/mini-player regressions and visual
smoke tests. A lockfile change also needs a new Nix vendor hash for the combined
lockfile, not upstream's hash. Nix validation runs in CI when Nix is unavailable
on the Mac. Install a verified Mac milestone without interrupting a running app.
