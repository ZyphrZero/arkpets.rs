#!/usr/bin/env bash
set -euo pipefail

# A private X server keeps automation away from the user's desktop.
if [[ ${1:-} != --inside ]]; then
    exec xvfb-run -a -s '-screen 0 1600x1200x24 +extension Composite' bash "$0" --inside "$@"
fi
shift
repository=$(cd "$(dirname "$0")/../.." && pwd)
executable=${1:-"$repository/target/debug/arkpets-ui"}
output=${2:-$(mktemp -d -t arkpets-shell-XXXXXX)}
mkdir -p "$output"
output=$(cd "$output" && pwd)
app_pid=
wm_pid=
compositor_pid=
cleanup() {
    for process in "$app_pid" "$compositor_pid" "$wm_pid"; do
        if [[ -n $process ]]; then
            kill "$process" 2>/dev/null || true
            wait "$process" 2>/dev/null || true
        fi
    done
}
trap cleanup EXIT
openbox --sm-disable >"$output/wm.log" 2>&1 &
wm_pid=$!
xcompmgr -n >"$output/compositor.log" 2>&1 &
compositor_pid=$!
for _ in {1..100}; do
    if xprop -root _NET_SUPPORTED | grep -q _NET_WM_STATE_SKIP_TASKBAR; then break; fi
    sleep 0.05
done
xprop -root _NET_SUPPORTED | grep -q _NET_WM_STATE_SKIP_TASKBAR
xsetroot -solid '#527c93'
cd "$repository"
ARKPETS_PROFILE_DIR="$output" ARKPETS_PROFILE_SECONDS=90 ARKPETS_PROFILE_WARMUP=2 \
    "$executable" --renderer vulkan >"$output/stdout.log" 2>"$output/stderr.log" &
app_pid=$!

find_window() {
    local name=$1 window
    for _ in {1..200}; do
        kill -0 "$app_pid" 2>/dev/null || { printf 'Application exited; see %s/stderr.log\n' "$output" >&2; return 1; }
        window=$(xdotool search --all --pid "$app_pid" --name "$name" 2>/dev/null | head -n 1 || true)
        if [[ -n $window ]]; then printf '%s\n' "$window"; return; fi
        sleep 0.05
    done
    printf 'Window did not appear: %s\n' "$name" >&2
    return 1
}
root=$(find_window '^ArkPets$')
pet=$(find_window '^ArkPets - ')

assert_state() {
    local hidden=$1 states matched=false
    for _ in {1..100}; do
        kill -0 "$app_pid"
        states=$(xprop -id "$pet" _NET_WM_STATE)
        if [[ $hidden == true && $states == *_NET_WM_STATE_SKIP_TASKBAR* && $states == *_NET_WM_STATE_SKIP_PAGER* ]] ||
           [[ $hidden == false && $states != *_NET_WM_STATE_SKIP_TASKBAR* && $states != *_NET_WM_STATE_SKIP_PAGER* ]]; then
            if xwininfo -id "$pet" | grep -q 'Map State: IsViewable'; then
                matched=true
                break
            fi
        fi
        sleep 0.05
    done
    if [[ $matched != true ]]; then
        printf 'Unexpected state: %s\n' "$states" >&2
        xwininfo -id "$pet"
        xprop -id "$pet" _NET_WM_WINDOW_TYPE _NET_WM_PID
        import -window root "$output/failure.png"
        return 1
    fi
    xwininfo -id "$pet" | grep -q 'Map State: IsViewable'
    [[ $(find_window '^ArkPets - ') == "$pet" ]]
    printf 'PASS: hidden=%s, visible X11 window=%s\n' "$hidden" "$pet"
}

assert_animation() {
    local mode=$1 colors difference status
    xdotool windowminimize "$root"
    sleep 1
    import -window "$pet" "$output/$mode-1.png"
    sleep 0.4
    import -window "$pet" "$output/$mode-2.png"
    colors=$(identify -format '%k' "$output/$mode-1.png")
    [[ $colors -gt 20 ]] || { printf 'Pet image is blank (%s colors)\n' "$colors" >&2; return 1; }
    status=0
    difference=$(compare -metric AE "$output/$mode-1.png" "$output/$mode-2.png" null: 2>&1) || status=$?
    [[ $status -eq 1 ]] || { printf 'Animation did not change, or capture failed: %s\n' "$difference" >&2; return 1; }
    printf 'PASS: %s animation changed %s pixels (%s colors)\n' "$mode" "$difference" "$colors"
    xdotool windowmap "$root" windowactivate --sync "$root"
}

assert_state true
assert_animation hidden
xdotool windowsize "$root" 1200 1000 windowmove "$root" 10 10
sleep 0.5 # Wait for the WM to apply geometry before translating pointer coordinates.
xdotool mousemove --window "$root" 700 500
sleep 0.1
xdotool click --repeat 40 --delay 20 5
sleep 0.5
for hidden in false true false; do
    xdotool mousemove --window "$root" 270 638 click 1
    assert_state "$hidden"
done
assert_animation shown
import -window root "$output/desktop.png"
wmctrl -i -c "$root"
for _ in {1..100}; do
    if ! kill -0 "$app_pid" 2>/dev/null; then break; fi
    sleep 0.05
done
if kill -0 "$app_pid" 2>/dev/null; then
    printf 'Application did not exit after WM_DELETE_WINDOW\n' >&2
    exit 1
fi
wait "$app_pid"
app_pid=
printf 'Linux shell visibility PASS. Artifacts: %s\n' "$output"
