#!/usr/bin/env sh
set -eu

printf '%s\n' '[security-post-action] review only; findings need proof'
printf '%s\n' '[security-post-action] see AGENT_HARNESS.md and scripts/security-watchlist.md'

git diff --unified=0 -- . \
  | rg -n '^\+.*(unsafe|unwrap\(|unwrap_unchecked|expect\(|panic!|todo!|Command::new|spawn\(|temp_dir\(|canonicalize\(|read_to_string\(|write\(|remove_file\()' \
  || true
