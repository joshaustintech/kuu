#!/usr/bin/env sh
set -u

cargo build --quiet
binary="$PWD/target/debug/kuu"
root="tests/upstream/lua-5.5.0-tests"
err_dir="$PWD/target"
failed=0

for script in "$root"/*.lua; do
  name=${script#"$root"/}
  script_abs="$PWD/$script"
  if (cd "$root" && "$binary" "$script_abs") >/dev/null 2>"$err_dir/kuu-smoke-$name.err"; then
    printf 'PASS %s\n' "$name"
  else
    printf 'FAIL %s\n' "$name"
    failed=$((failed + 1))
  fi
done

printf 'SUMMARY total=%s failed=%s\n' "$(find "$root" -maxdepth 1 -name '*.lua' | wc -l | tr -d ' ')" "$failed"
exit "$failed"
