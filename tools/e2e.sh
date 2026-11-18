#!/usr/bin/env bash
# In-game check for DimStorage slice 1 (the dimensional chest).
#
# The Overworld arena at y=100 (stone floor at y=99). Every mod command is
# typed with `ctl chat "/cmd"` because `ctl command` runs the console path and
# bypasses the native `run_mod_command` hook.
#
# Default run (a game the launcher built with this package, which carries both
# the code and the content):
#  - two chests on the default public/1 frequency start empty;
#  - a stack deposited through the 54-slot menu appears in both chests;
#  - one item withdrawn through the menu leaves both with the rest;
#  - a chest moved to channel 2 is isolated, and a third on channel 3 is empty;
#  - a vanilla barrel with sticks never shares with either chest;
#  - the store file exists and all three frequencies survive a relaunch.
#
# SELFTEST=1 flips the sharing assertion (it must FAIL), proving the scenario
# can fail.
#
# A native `.hmod` cannot load on an unmodded game, so GAME must name a game
# the launcher built with this package:
#   horizon-launcher native-build ports/dimstorage --instance <i> --trust-native-code
# CONTROL_GAME=<path> adds a differential leg: the same scenario's first two
# checks on a game without the package, where both ids must be unknown.
# With a game built from the sources instead, set CONTENT_DIR to the package
# root (it has horizon-mod.toml + assets/ + data/) and pass `--dev-mod`.
set -euo pipefail
export LC_ALL=C

CLONE=/home/adlf/Desktop/all_the_mods_10_rust_rewrite
PORT=$CLONE/ports/dimstorage
GAME=${GAME:-}
[ -n "$GAME" ] || {
    echo "GAME must name a game the launcher built with this package (horizon-launcher native-build ports/dimstorage …)" >&2
    exit 1
}
CTL=${CTL:-$(dirname "$GAME")/horizon-ctl}
CONTROL_GAME=${CONTROL_GAME:-}
CONTENT_DIR=${CONTENT_DIR:-}
DEV_MOD=()
[ -z "$CONTENT_DIR" ] || DEV_MOD=(--dev-mod "$CONTENT_DIR")
RES=${RES:-${HORIZON_RES:-/home/adlf/Desktop/horizon/.local/horizon-client/26.3}}
SOCK=${SOCK:-/tmp/opencode/dimstorage-e2e.sock}
SELFTEST=${SELFTEST:-0}
KEEP=${KEEP:-0}
WORLD=DimStorageE2E
CONTROL_WORLD=DimStorageControl
BLOCK=dimstorage:dimensional_chest

[ -x "$CTL" ] || { echo "missing horizon-ctl: $CTL (set CTL=)" >&2; exit 1; }
[ -x "$GAME" ] || { echo "missing horizon-client: $GAME (set GAME=)" >&2; exit 1; }
[ -d "$RES/assets" ] || { echo "missing prepared resources: $RES" >&2; exit 1; }

SCRATCH=$(mktemp -d /tmp/opencode/dimstorage-home.XXXXXX)
export HORIZON_HOME=$SCRATCH/game
export HORIZON_LAUNCHER_HOME=$SCRATCH/launcher
export XDG_DATA_HOME=$SCRATCH/data
export XDG_CONFIG_HOME=$SCRATCH/config
export HORIZON_NO_AUDIO=1
mkdir -p "$(dirname "$SOCK")" "$XDG_CONFIG_HOME"
rm -f "$SOCK"

ctl() { "$CTL" --socket "$SOCK" "$@"; }
say() { ctl chat "/$1" >/dev/null; }
fail() { echo "FAIL: $*" >&2; ctl logs --lines 80 >&2 || true; exit 1; }

# The non-null slots of the container at a position, as JSON.
slots() { ctl container "$1" "$2" "$3" | jq -c '[.slots[]? | select(. != null)]'; }
# has X Y Z ITEM COUNT: the container at the position holds exactly COUNT of ITEM.
has() { # has X Y Z ITEM COUNT
    local got
    got=$(slots "$1" "$2" "$3")
    jq -e --arg item "$4" --argjson count "$5" \
        '[.[] | select(.item == $item) | .count] | add // 0 | . == $count' <<<"$got" >/dev/null \
        || fail "expected $5 x $4 at $1,$2,$3; got $got"
    echo "  has($1,$2,$3) $4 x$5: ok"
}
empty() { # empty X Y Z
    local got
    got=$(slots "$1" "$2" "$3")
    jq -e 'length == 0' <<<"$got" >/dev/null \
        || fail "expected an empty container at $1,$2,$3; got $got"
    echo "  empty($1,$2,$3): ok"
}
carried_count() { ctl menu | jq -r '.carried.count // 0'; }
inv_count() { # inv_count ITEM
    ctl inventory | jq -r --arg item "$1" \
        '[((.hotbar // [])[]), ((.main // [])[])] | map(select(. != null and .item == $item)) | map(.count) | add // 0'
}
info() { # info X Y Z: the /dimstorage info line
    say "dimstorage info $1 $2 $3"
    ctl wait 4
    ctl messages 8 | jq -r '.chat[]?' | grep -m1 'DimStorage:' || true
}
expect_info() { # expect_info X Y Z TEXT
    local line
    line=$(info "$1" "$2" "$3")
    echo "  info($1,$2,$3): $line"
    case "$line" in
        *"$4"*) ;;
        *) fail "expected /dimstorage info to say '$4', got '$line'" ;;
    esac
}

cleanup() {
    local status=$?
    ctl quit >/dev/null 2>&1 || true
    sleep 1
    if [ "$KEEP" = 1 ]; then
        echo "kept scratch: $SCRATCH" >&2
    else
        rm -rf "$SCRATCH" 2>/dev/null || true
    fi
    # Last line of every run: `check-e2e-logs.py` reads it as the verdict.
    echo "EXIT:$status"
}
trap cleanup EXIT

launch() { # launch [WORLD] [GAME]
    local world=${1:-$WORLD}
    local game=${2:-$GAME}
    rm -f "$SOCK"
    cd "$SCRATCH"
    "$CTL" --socket "$SOCK" launch \
        --game "$game" --world "$world" --seed 20261105 --creative --offscreen --wait \
        --resources "$RES" "${DEV_MOD[@]}" >/dev/null
    ctl wait-ready 120
    ctl wait 40
}

# The first launch only: a stone floor and a clear volume in the Overworld.
arena() {
    ctl command "execute in minecraft:overworld run tp @s 0.5 100 3.5"
    local loaded=false
    for _ in $(seq 1 120); do
        ctl wait 5
        if [ "$(ctl block 0 99 0 | jq -r '.loaded // false')" = true ]; then
            loaded=true
            break
        fi
    done
    [ "$loaded" = true ] || fail "the Overworld arena chunk never loaded"
    ctl command "execute in minecraft:overworld run fill -8 99 -8 24 99 24 minecraft:stone"
    ctl command "execute in minecraft:overworld run fill -8 100 -8 24 110 24 minecraft:air"
    ctl command "execute in minecraft:overworld run kill @e[type=!player]"
    ctl wait 20
}

# The differential leg: the same scenario's first checks on a game built
# WITHOUT this package. Both ids must be unknown there; if they resolve, the
# main run proves nothing about the package.
control_leg() {
    echo "== control: the same scenario on a game without this package =="
    launch "$CONTROL_WORLD" "$CONTROL_GAME"
    arena
    ctl command "item replace entity @s hotbar.0 with dimstorage:dimensional_chest 1" >/dev/null || true
    ctl wait 4
    ctl inventory | jq -e --arg id "dimstorage:dimensional_chest" \
        '[.hotbar[], .main[] | select(.item == $id)] | length == 0' >/dev/null \
        || fail "the control game has the chest item, so the id was known without the package"
    ctl command "execute in minecraft:overworld run setblock 20 100 0 $BLOCK" >/dev/null || true
    ctl wait 4
    local name
    name=$(ctl block 20 100 0 | jq -r '.name // .block // empty')
    [ "$name" = "minecraft:air" ] || fail "the control game holds $name, so the block id was known without the package"
    echo "  without the package: no chest item, the block stayed $name"
    ctl quit >/dev/null 2>&1 || true
}

echo "== launch (Overworld arena, the package carries code and content) =="
if [ -n "$CONTROL_GAME" ]; then control_leg; fi
launch
arena

echo "== four chests start empty; channels 2 and 3 are separate =="
ctl command "execute in minecraft:overworld run setblock 0 100 0 $BLOCK"
ctl command "execute in minecraft:overworld run setblock 4 100 0 $BLOCK"
ctl command "execute in minecraft:overworld run setblock 8 100 0 $BLOCK"
ctl command "execute in minecraft:overworld run setblock 16 100 0 $BLOCK"
ctl wait 4
say "dimstorage channel 8 100 0 2"
say "dimstorage channel 16 100 0 3"
ctl wait 10
expect_info 0 100 0 "public / 1"
expect_info 8 100 0 "public / 2"
expect_info 16 100 0 "public / 3"
empty 0 100 0
empty 4 100 0
empty 8 100 0
empty 16 100 0

echo "== deposit 8 diamonds through the 54-slot menu =="
ctl command "execute in minecraft:overworld run tp @s 0.5 100 2.5"
ctl wait 6
ctl command "item replace entity @s hotbar.0 with minecraft:diamond 8"
ctl wait 4
ctl select minecraft:diamond
ctl wait 4
ctl interact 0 100 0
ctl wait-for screen container --timeout 10 >/dev/null \
    || fail "the chest did not open a menu (screen=$(ctl state | jq -r '.screen // ""'))"
menu_slots=$(ctl screen | jq '[.slots[] | select(.slot >= 300)] | length')
echo "menu slots: $menu_slots"
[ "$menu_slots" = 54 ] || fail "the chest menu shows $menu_slots slots, expected 54"
ctl click-slot 0 >/dev/null || fail "cannot pick up the hotbar stack"
ctl wait 2
ctl click-slot 300 >/dev/null || fail "cannot deposit into the chest slot"
ctl wait 4
carried=$(carried_count)
[ "$carried" = 0 ] || fail "the cursor still carries $carried after the deposit"
ctl close >/dev/null 2>&1 || true
ctl wait 10
if [ "$SELFTEST" = 1 ]; then
    # Deliberately wrong: B is on the same frequency, so it must hold them.
    echo "(selftest: asserting the shared chest is empty)"
    empty 4 100 0
else
    has 4 100 0 minecraft:diamond 8
fi
has 0 100 0 minecraft:diamond 8
empty 8 100 0
empty 16 100 0
echo "A=$(slots 0 100 0)"
echo "B=$(slots 4 100 0)"

echo "== withdraw one diamond through the menu =="
ctl interact 0 100 0
ctl wait-for screen container --timeout 10 >/dev/null \
    || fail "the chest did not reopen its menu"
# Take the whole stack, place one in the hotbar with a right-click, put the
# remaining seven back. The menu's withdrawal is the left-click take; the
# right-click is vanilla's one-item placement.
ctl click-slot 300 >/dev/null || fail "cannot take the chest stack"
ctl wait 3
carried=$(carried_count)
echo "  carried after take: $carried"
[ "$carried" = 8 ] || fail "the menu took $carried diamonds, expected 8"
ctl click-slot 0 right >/dev/null || fail "cannot place one diamond in the hotbar"
ctl wait 3
carried=$(carried_count)
echo "  carried after placing one: $carried"
[ "$carried" = 7 ] || fail "the menu left $carried diamonds on the cursor, expected 7"
ctl click-slot 300 >/dev/null || fail "cannot put the remaining diamonds back"
ctl wait 3
carried=$(carried_count)
[ "$carried" = 0 ] || fail "the cursor still carries $carried after the withdrawal"
ctl close >/dev/null 2>&1 || true
ctl wait 10
has 0 100 0 minecraft:diamond 7
has 4 100 0 minecraft:diamond 7
one=$(inv_count minecraft:diamond)
echo "  diamond in the inventory: $one"
[ "$one" = 1 ] || fail "the menu gave $one diamonds, expected 1"

echo "== a channel-2 chest is isolated =="
ctl container 8 100 0 minecraft:emerald 3
ctl wait 8
has 8 100 0 minecraft:emerald 3
has 0 100 0 minecraft:emerald 0
has 8 100 0 minecraft:diamond 0
empty 16 100 0

echo "== a vanilla barrel never shares =="
ctl command "execute in minecraft:overworld run setblock 12 100 0 minecraft:barrel"
ctl wait 4
ctl container 12 100 0 minecraft:stick 7
ctl wait 8
has 12 100 0 minecraft:stick 7
has 0 100 0 minecraft:stick 0
has 8 100 0 minecraft:stick 0
has 12 100 0 minecraft:diamond 0
has 12 100 0 minecraft:emerald 0
echo "  barrel=$(slots 12 100 0)"

echo "== the store file exists and survives a relaunch =="
ctl wait 40
STORE="$SCRATCH/saves/$WORLD/dimstorage/inventories.dat"
[ -s "$STORE" ] || fail "missing store file $STORE"
grep -q 'f public,1' "$STORE" || fail "the store file has no public/1 frequency: $(head -20 "$STORE")"
grep -q 'f public,2' "$STORE" || fail "the store file has no public/2 frequency: $(head -20 "$STORE")"
grep -q 'f public,3' "$STORE" || fail "the store file has no public/3 frequency: $(head -20 "$STORE")"
echo "  store: $(grep '^f ' "$STORE" | tr '\n' ' ')"
ctl save >/dev/null
ctl quit >/dev/null 2>&1 || true
sleep 3
launch
has 0 100 0 minecraft:diamond 7
has 4 100 0 minecraft:diamond 7
has 8 100 0 minecraft:emerald 3
empty 16 100 0
expect_info 0 100 0 "public / 1"
expect_info 8 100 0 "public / 2"

echo "PASS: same-frequency chests shared 54 slots, deposits and withdrawals went through the menu, a channel-2 chest stayed isolated, an empty channel-3 chest stayed empty, a vanilla barrel never shared, and all three stores survived a relaunch"
