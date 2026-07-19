#!/usr/bin/env sh
set -u

cargo build --quiet
binary="$PWD/target/debug/kuu"
root="tests/upstream/lua-5.5.0-tests"
err_dir="$PWD/target"
failed=0
timeout_seconds=${KUU_SMOKE_TIMEOUT_SECONDS:-10}

run_with_timeout() {
  perl -e '
    $seconds = shift;
    $pid = fork;
    die "fork failed\n" unless defined $pid;
    if ($pid == 0) { exec @ARGV; die "exec failed\n"; }
    $SIG{ALRM} = sub { kill "TERM", $pid; waitpid $pid, 0; exit 124; };
    alarm $seconds;
    waitpid $pid, 0;
    exit($? >> 8);
  ' "$timeout_seconds" "$binary" "$1" >/dev/null 2>"$2"
}

for script in "$root"/*.lua; do
  name=${script#"$root"/}
  script_abs="$PWD/$script"
  if (cd "$root" && run_with_timeout "$script_abs" "$err_dir/kuu-smoke-$name.err"); then
    printf 'PASS %s\n' "$name"
  else
    status=$?
    if [ "$status" -eq 124 ]; then
      printf 'FAIL %s (timeout)\n' "$name"
    else
      printf 'FAIL %s\n' "$name"
    fi
    failed=$((failed + 1))
  fi
done

printf 'SUMMARY total=%s failed=%s\n' "$(find "$root" -maxdepth 1 -name '*.lua' | wc -l | tr -d ' ')" "$failed"
exit "$failed"
