#!/bin/zsh
# Phase 0 performance baseline (docs/plans/phase-0-spike.md section C).
#
# Every number this produces is an observation on one machine, never an
# assertion, and nothing here runs in CI. Read the warning at the top of
# section C before adding a threshold anywhere.
#
# Usage:
#   scripts/measure-phase-0.sh build            # release bundle
#   scripts/measure-phase-0.sh launch <label> <datadir> <runs> [keep]
#   scripts/measure-phase-0.sh sample <label> <datadir> <seconds> <interval>
#   scripts/measure-phase-0.sh electron <label> <workspace> <seconds> <interval>
#
# Results are appended to $OUT (default: /tmp/gritty-measure).

set -u
REPO="${GRITTY_REPO:-/Users/alexistan/Desktop/CurrentProjects/GrittyIDE}"
APP="$REPO/target/release/bundle/macos/Gritty.app/Contents/MacOS/gritty-desktop"
OUT="${OUT:-/tmp/gritty-measure}"
mkdir -p "$OUT"

# Every WebKit helper process on the machine, as "pid comm".
webkit_pids() {
  ps -A -o pid=,comm= | grep -i "com.apple.WebKit" | awk '{print $1}' | sort
}

# Seed a session so the app restores a workspace (and tabs) without anyone
# clicking. The app creates and migrates the database itself on a first
# launch; this only inserts rows into the schema it made.
seed_session() {
  local datadir="$1" root="$2"
  local db="$datadir/gritty.sqlite"
  local now=$(date +%s000)
  [[ -f "$db" ]] || { echo "no database at $db (launch once first)"; return 1; }
  sqlite3 "$db" <<SQL
INSERT OR REPLACE INTO workspace(root, last_opened_at) VALUES ('$root', $now);
INSERT OR REPLACE INTO session(id, workspace_root, editor_preferences, layout, updated_at)
  VALUES (1, '$root', NULL, NULL, $now);
SQL
}

seed_tabs() {
  local datadir="$1" root="$2"; shift 2
  local db="$datadir/gritty.sqlite"
  local now=$(date +%s000) pos=0
  # Not `for path in`: `path` is tied to `PATH` in zsh, and assigning it here
  # empties the command search path for the rest of the function.
  for tab in "$@"; do
    local active=0
    [[ $pos -eq 0 ]] && active=1
    sqlite3 "$db" "INSERT OR REPLACE INTO workspace_tab(workspace_root, path, position, active, view_mode, cursor_line, cursor_column, updated_at) VALUES ('$root', '$tab', $pos, $active, NULL, 1, 1, $now);"
    pos=$((pos + 1))
  done
}

# One launch: start the app, wait for the last startup milestone, record it,
# then stop. SIGTERM skips RunEvent::Exit, which is fine here because nothing
# in these runs depends on a clean shutdown flush.
one_launch() {
  local label="$1" datadir="$2" run="$3" logenv="${4:-}"
  local log="$OUT/$label-$run.log"
  local before after helpers
  before=$(webkit_pids)
  if [[ -n "$logenv" ]]; then
    GRITTY_DATA_DIR="$datadir" GRITTY_LOG="$logenv" "$APP" > "$log" 2>&1 &
  else
    GRITTY_DATA_DIR="$datadir" "$APP" > "$log" 2>&1 &
  fi
  local pid=$!
  local i
  for i in $(seq 1 40); do
    grep -q "first webview health" "$log" 2>/dev/null && break
    sleep 0.25
  done
  sleep 1
  after=$(webkit_pids)
  helpers=$(comm -13 <(echo "$before") <(echo "$after") | tr '\n' ' ')
  echo "run=$run pid=$pid helpers=[$helpers]"
  grep "startup milestone" "$log" | sed -E 's/.*milestone="([^"]+)".*elapsed_ms=([0-9]+).*/  \1 = \2 ms/'
  kill "$pid" 2>/dev/null
  wait "$pid" 2>/dev/null
  sleep 1
}

# CPU and memory of Gritty plus the WebKit helpers it spawned, sampled over
# time. Summing the helpers is the easy thing to get wrong: WKWebView runs the
# renderer out of process, so the app's own RSS is not the footprint.
sample_run() {
  local label="$1" datadir="$2" seconds="$3" interval="$4"
  local log="$OUT/$label-sample.log" csv="$OUT/$label-sample.csv"
  local before after helpers
  before=$(webkit_pids)
  GRITTY_DATA_DIR="$datadir" "$APP" > "$log" 2>&1 &
  local pid=$!
  local i
  for i in $(seq 1 40); do
    grep -q "first webview health" "$log" 2>/dev/null && break
    sleep 0.25
  done
  sleep 2
  after=$(webkit_pids)
  helpers=$(comm -13 <(echo "$before") <(echo "$after") | tr '\n' ' ')
  echo "app=$pid helpers=[$helpers]"
  # `ps -o %cpu` is an average over the whole process lifetime, which is not
  # idle CPU. Record cumulative CPU time instead and take deltas afterwards.
  # RSS is not a growth metric for an idle app on macOS: the OS reclaims
  # untouched pages, so resident memory *falls* while nothing is allocated.
  # `Physical footprint` (what Activity Monitor calls Memory) counts the
  # compressed and swapped pages too, so record both.
  echo "t,pid,cputime,rss_kb,footprint_mb" > "$csv"
  for p in $pid ${=helpers}; do
    echo "$p,$(ps -o ucomm= -p $p 2>/dev/null)" >> "$OUT/$label-names.csv"
  done
  local elapsed=0
  while [[ $elapsed -lt $seconds ]]; do
    for p in $pid ${=helpers}; do
      local foot
      foot=$(vmmap --summary "$p" 2>/dev/null \
        | awk '/Physical footprint:/ {gsub(/[MG]/,"",$3); print $3; exit}')
      ps -o pid=,time=,rss= -p "$p" 2>/dev/null \
        | awk -v t="$elapsed" -v f="${foot:-}" '{print t","$1","$2","$3","f}' >> "$csv"
    done
    sleep "$interval"
    elapsed=$((elapsed + interval))
  done
  kill "$pid" 2>/dev/null
  echo "wrote $csv"
}

# The Electron comparison for section C. Cursor is what is installed here; it
# runs with extensions disabled and a throwaway user-data directory so the
# measurement neither depends on nor disturbs the real profile. It is a full
# IDE, not an empty Electron shell, so this is an upper bound on "an Electron
# editor with this workspace open", not a measurement of Electron itself.
electron_run() {
  local label="$1" workspace="$2" seconds="$3" interval="$4"
  local csv="$OUT/$label-sample.csv"
  pkill -f "user-data-dir=/tmp/cursor-measure" 2>/dev/null
  rm -rf /tmp/cursor-measure
  open -na Cursor --args --disable-extensions --disable-workspace-trust \
    --user-data-dir=/tmp/cursor-measure --extensions-dir=/tmp/cursor-measure-ext \
    "$workspace"
  sleep 25
  local pids
  pids=$(pgrep -f "user-data-dir=/tmp/cursor-measure" | tr '\n' ' ')
  echo "cursor pids=[$pids]"
  echo "t,pid,cputime,rss_kb" > "$csv"
  rm -f "$OUT/$label-names.csv"
  for p in ${=pids}; do
    echo "$p,$(ps -o ucomm= -p $p 2>/dev/null)" >> "$OUT/$label-names.csv"
  done
  local elapsed=0
  while [[ $elapsed -lt $seconds ]]; do
    for p in ${=pids}; do
      ps -o pid=,time=,rss= -p "$p" 2>/dev/null \
        | awk -v t="$elapsed" '{print t","$1","$2","$3}' >> "$csv"
    done
    sleep "$interval"
    elapsed=$((elapsed + interval))
  done
  pkill -f "user-data-dir=/tmp/cursor-measure" 2>/dev/null
  echo "wrote $csv"
}

case "${1:-}" in
  build)
    export PATH="/opt/homebrew/opt/rustup/bin:$PATH"
    cd "$REPO" && pnpm tauri build --bundles app
    ;;
  launch)
    # A fifth argument means "keep the data directory": that is the difference
    # between a cold launch (nothing to restore) and a warm one.
    label="$2"; datadir="$3"; runs="$4"; keep="${5:-}"
    for r in $(seq 1 "$runs"); do
      [[ -z "$keep" ]] && rm -rf "$datadir"
      one_launch "$label" "$datadir" "$r" "${GRITTY_LOG:-}"
    done
    ;;
  sample)
    sample_run "$2" "$3" "$4" "$5"
    ;;
  electron)
    electron_run "$2" "$3" "$4" "$5"
    ;;
  seed)
    datadir="$2"; root="$3"; shift 3
    seed_session "$datadir" "$root" || exit 1
    # Not `[[ ... ]] && seed_tabs`: as the last statement that leaves the
    # script with status 1 when there are no tabs, which reads as a failure.
    if [[ $# -gt 0 ]]; then seed_tabs "$datadir" "$root" "$@"; fi
    ;;
  *)
    sed -n '2,20p' "$0"
    ;;
esac
