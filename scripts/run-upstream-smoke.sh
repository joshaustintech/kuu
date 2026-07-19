#!/usr/bin/env sh
set -u

cargo build --quiet
binary="$PWD/target/debug/kuu"
root="tests/upstream/lua-5.5.0-tests"
err_dir="$PWD/target"
failed=0
timeout_seconds=${KUU_SMOKE_TIMEOUT_SECONDS:-10}
active_pid=

stop_active_run() {
  if [ -n "$active_pid" ]; then
    kill -TERM "$active_pid" 2>/dev/null || true
    wait "$active_pid" 2>/dev/null || true
  fi
  exit 143
}

trap stop_active_run HUP INT TERM

run_with_timeout() {
  perl -e '
    use POSIX qw(WNOHANG);
    $seconds = shift;
    $pid = fork;
    die "fork failed\n" unless defined $pid;
    if ($pid == 0) { exec @ARGV; die "exec failed\n"; }
    sub stop_child {
      kill "TERM", $pid;
      for (1..20) {
        my $result = waitpid $pid, WNOHANG;
        return if $result == $pid;
        select undef, undef, undef, 0.05;
      }
      kill "KILL", $pid;
      waitpid $pid, 0;
    }
    $SIG{ALRM} = sub { stop_child(); exit 124; };
    $SIG{HUP} = sub { stop_child(); exit 129; };
    $SIG{INT} = sub { stop_child(); exit 130; };
    $SIG{TERM} = sub { stop_child(); exit 143; };
    alarm $seconds;
    waitpid $pid, 0;
    exit($? >> 8);
  ' "$timeout_seconds" "$binary" "$1" >/dev/null 2>"$2" &
  active_pid=$!
  wait "$active_pid"
  status=$?
  active_pid=
  return "$status"
}

for script in "$root"/*.lua; do
  name=${script#"$root"/}
  script_abs="$PWD/$script"
  original_dir=$PWD
  cd "$root" || exit 1
  if run_with_timeout "$script_abs" "$err_dir/kuu-smoke-$name.err"; then
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
  cd "$original_dir" || exit 1
done

printf 'SUMMARY total=%s failed=%s\n' "$(find "$root" -maxdepth 1 -name '*.lua' | wc -l | tr -d ' ')" "$failed"
exit "$failed"
