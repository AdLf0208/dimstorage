#!/usr/bin/env bash
# In-game check for DimStorage: the dimensional chest (slice 1) and the
# dimensional tank (slice 2), on a game the launcher built with this package.
#
# The Overworld arena at y=100 (stone floor at y=99). Every mod command is
# typed with `ctl chat "/cmd"` because `ctl command` runs the console path and
# bypasses the native `run_mod_command` hook.
#
# Default run (a game the launcher built with this package, which carries both
# the code and the content):
#  - the tank recipe crafts in a crafting table (output id and count, and the
#    ingredients consumed), and the crafted item places the block;
#  - two chests on the default public/1 frequency start empty;
#  - a stack deposited through the 54-slot menu appears in both chests;
#  - one item withdrawn through the menu leaves both with the rest;
#  - a chest moved to channel 2 is isolated, and a third on channel 3 is empty;
#  - a vanilla barrel with sticks never shares with either chest;
#  - a water bucket empties into a tank and every tank on its channel shows it;
#  - a channel-2 tank holds lava without touching channel 1's water;
#  - a tank channel caps at 16000 mB; the 17th bucket is refused before it moves;
#  - a drain through one tank is committed at access: with the clock frozen and the
#    sibling removed before its next tick, a relaunch keeps exactly the drained
#    amount (the channel is not restored from a stale saved compound);
#  - breaking a tank drops it into a hopper and leaves the channel's fluid intact;
#  - both store files exist and both stores survive a relaunch.
#
# SELFTEST=1 flips the chest sharing assertion; SELFTEST=tank flips the tank
# sharing assertion; SELFTEST=capacity flips the refused-fill assertion;
# SELFTEST=transfer flips the drain-committed assertion. Each must FAIL (EXIT:1),
# proving the scenario can fail.
#
# A native `.hmod` cannot load on an unmodded game, so GAME must name a game
# the launcher built with this package:
#   horizon-launcher native-build ports/dimstorage --instance <i> --trust-native-code
# CONTROL_GAME=<path> adds a differential leg: the same ids on a game without
# the package, where they must be unknown.
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
# `/tmp` is a small tmpfs; the launcher's env names a scratch folder on /home.
E2E_TMP=${E2E_TMP:-/tmp/opencode}
mkdir -p "$E2E_TMP"
SOCK=${SOCK:-$E2E_TMP/dimstorage-e2e.sock}
SELFTEST=${SELFTEST:-0}
KEEP=${KEEP:-0}
WORLD=DimStorageE2E
CONTROL_WORLD=DimStorageControl
BLOCK=dimstorage:dimensional_chest
TANK=dimstorage:dimensional_tank

[ -x "$CTL" ] || { echo "missing horizon-ctl: $CTL (set CTL=)" >&2; exit 1; }
[ -x "$GAME" ] || { echo "missing horizon-client: $GAME (set GAME=)" >&2; exit 1; }
[ -d "$RES/assets" ] || { echo "missing prepared resources: $RES" >&2; exit 1; }

SCRATCH=$(mktemp -d "$E2E_TMP/dimstorage-home.XXXXXX")
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
carried_item() { ctl menu | jq -r '.carried.item // ""'; }
inv_count() { # inv_count ITEM
    ctl inventory | jq -r --arg item "$1" \
        '[((.hotbar // [])[]), ((.main // [])[])] | map(select(. != null and .item == $item)) | map(.count) | add // 0'
}
stand() { # stand X Y Z: an ordinary player two blocks diagonally from the block, in reach
    ctl command "execute in minecraft:overworld run tp @s $(($1 + 2)).5 $2 $(($3 + 2)).5" >/dev/null
    ctl wait 3 >/dev/null
}
info() { # info X Y Z: the /dimstorage info line
    # Since 0.2.1 the channel commands require physical access (the channel is the key to a shared
    # store), so the harness player stands beside the block first. No operator is used, so the run
    # proves the ordinary-player path.
    stand "$1" "$2" "$3"
    say "dimstorage info $1 $2 $3"
    ctl wait 4 >/dev/null
    ctl messages 8 | jq -r '.chat[]?' | grep -m1 'DimStorage' || true
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
# expect_tank X Y Z TEXT: the tank readout at a position (same command, tank block).
expect_tank() { expect_info "$@"; }
# The tank's own saved compound (its `fluids` path), spaces removed so the SNBT
# shape (`{Amount: 15000, Fluid: "minecraft:water"}`) is easy to assert on.
tank_compound() { # tank_compound X Y Z
    ctl command "data get block $1 $2 $3 fluids" | jq -r '.chat[]?' | head -1 | tr -d ' '
}

# Fill the tank at X Y Z from the hotbar item (a bucket), then let the store settle.
use_on() { # use_on ITEM X Y Z
    ctl select "$1" >/dev/null || fail "the hotbar holds no $1 to use"
    ctl wait 3 >/dev/null
    ctl interact "$2" "$3" "$4" >/dev/null || fail "using $1 on $2,$3,$4 failed"
    ctl wait 8 >/dev/null
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

# The differential leg: the same ids on a game built WITHOUT this package. Both
# must be unknown there; if they resolve, the main run proves nothing.
control_leg() {
    echo "== control: the same scenario on a game without this package =="
    launch "$CONTROL_WORLD" "$CONTROL_GAME"
    arena
    local id
    for id in "$BLOCK" "$TANK"; do
        ctl command "item replace entity @s hotbar.0 with $id 1" >/dev/null || true
        ctl wait 4
        ctl inventory | jq -e --arg id "$id" \
            '[.hotbar[], .main[] | select(.item == $id)] | length == 0' >/dev/null \
            || fail "the control game has the $id item, so the id was known without the package"
        ctl command "execute in minecraft:overworld run setblock 20 100 0 $id" >/dev/null || true
        ctl wait 4
        local name
        name=$(ctl block 20 100 0 | jq -r '.name // .block // empty')
        [ "$name" = "minecraft:air" ] || fail "the control game holds $name, so $id was known without the package"
        echo "  without the package: no $id item, the block stayed $name"
    done
    ctl quit >/dev/null 2>&1 || true
}

# Craft the dimensional tank in a crafting table: the 3x3 grid is slots 100-108
# (row by row) and the result is 200. Asserts the output id and count and that
# every ingredient left the grid.
craft_tank() {
    echo "== the tank recipe crafts in a crafting table =="
    ctl command "execute in minecraft:overworld run setblock 22 100 0 minecraft:crafting_table"
    ctl wait 4
    ctl command "execute in minecraft:overworld run tp @s 22.5 100 2.5"
    ctl wait 4
    ctl command "item replace entity @s hotbar.0 with dimstorage:dim_wall 4"
    ctl command "item replace entity @s hotbar.1 with minecraft:glass 3"
    ctl command "item replace entity @s hotbar.2 with minecraft:cauldron 1"
    ctl command "item replace entity @s hotbar.3 with dimstorage:solid_dim_core 1"
    ctl wait 6
    ctl interact 22 100 0
    # Horizon shows a crafting table's 3x3 menu in the inventory screen: the grid
    # is slots 100-108 (row by row) and the result is 200.
    ctl wait-for screen inventory --timeout 10 >/dev/null \
        || fail "the crafting table did not open a menu (screen=$(ctl state | jq -r '.screen // ""'))"
    echo "  crafting slots: $(ctl screen | jq -c '[.slots[].slot]')"
    # The pattern is  a d a / d c d / a b a  with the tank's own recipe keys:
    # a = dim_wall, d = glass (the c:glass_blocks tag), c = cauldron, b = solid_dim_core.
    # A left click drops the whole carried stack into a grid slot, so a stack of
    # several ingredients goes in one item per right click.
    local a_slots="100 102 106 108" d_slots="101 103 105" slot
    ctl click-slot 0 >/dev/null || fail "cannot pick up the dim walls"
    ctl wait 2
    for slot in $a_slots; do ctl click-slot "$slot" right >/dev/null; ctl wait 2; done
    ctl click-slot 1 >/dev/null || fail "cannot pick up the glass"
    ctl wait 2
    for slot in $d_slots; do ctl click-slot "$slot" right >/dev/null; ctl wait 2; done
    ctl click-slot 2 >/dev/null || fail "cannot pick up the cauldron"
    ctl wait 2
    ctl click-slot 104 >/dev/null; ctl wait 2
    ctl click-slot 3 >/dev/null || fail "cannot pick up the solid dim core"
    ctl wait 2
    ctl click-slot 107 >/dev/null; ctl wait 2
    local result
    result=$(ctl screen | jq -c '[.slots[] | select(.slot == 200)][0] // {}')
    echo "  result slot: $result"
    jq -e --arg id "$TANK" '.item == $id and .count == 1' <<<"$result" >/dev/null \
        || fail "the crafting table offered $result, expected 1 x $TANK"
    ctl click-slot 200 >/dev/null || fail "cannot take the crafted tank"
    ctl wait 3
    local took
    took=$(carried_item)
    [ "$took" = "$TANK" ] || fail "the cursor carries '$took' after the craft, expected $TANK"
    ctl click-slot 4 >/dev/null || fail "cannot put the crafted tank in the hotbar"
    ctl wait 3
    # One of each ingredient was used: the grid is empty and the inventory holds one tank.
    local left
    left=$(ctl screen | jq -c '[.slots[] | select(.slot >= 100 and .slot <= 108 and .item != null)]')
    [ "$left" = "[]" ] || fail "the crafting grid still holds $left after the craft"
    echo "  the grid is empty after the craft (every ingredient was consumed)"
    ctl close >/dev/null 2>&1 || true
    ctl wait 6
    local tanks
    tanks=$(inv_count "$TANK")
    echo "  dimensional tanks in the inventory: $tanks"
    [ "$tanks" = 1 ] || fail "the craft left $tanks tanks, expected 1"
    # The crafted item places the block (the player must be within reach, and not
    # standing in the target).
    stand 2 100 12
    ctl place 2 100 12 --item "$TANK" >/dev/null || fail "cannot place the crafted tank"
    ctl wait 6
    local placed_json placed
    placed_json=$(ctl block 2 100 12)
    echo "  block at 2,100,12: $placed_json"
    # A mod block reports `.block`; a vanilla one `.name`.
    placed=$(jq -r '.block // .name // .state // empty' <<<"$placed_json")
    [ "$placed" = "$TANK" ] || fail "the crafted tank placed '$placed' at 2,100,12"
    echo "  the crafted item placed $placed at 2,100,12"
}

echo "== launch (Overworld arena, the package carries code and content) =="
if [ -n "$CONTROL_GAME" ]; then control_leg; fi
launch
arena
craft_tank

echo "== four chests start empty; channels 2 and 3 are separate =="
ctl command "execute in minecraft:overworld run setblock 0 100 0 $BLOCK"
ctl command "execute in minecraft:overworld run setblock 4 100 0 $BLOCK"
ctl command "execute in minecraft:overworld run setblock 8 100 0 $BLOCK"
ctl command "execute in minecraft:overworld run setblock 16 100 0 $BLOCK"
ctl wait 4
# The channel command needs the player at the chest since 0.2.1.
ctl command "execute in minecraft:overworld run tp @s 10.5 100 2.5" >/dev/null
ctl wait 3
say "dimstorage channel 8 100 0 2"
ctl wait 6
ctl command "execute in minecraft:overworld run tp @s 18.5 100 2.5" >/dev/null
ctl wait 3
say "dimstorage channel 16 100 0 3"
ctl command "execute in minecraft:overworld run tp @s 0.5 100 3.5" >/dev/null
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
if [ "$SELFTEST" = 1 ] || [ "$SELFTEST" = chest ]; then
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

echo "== the tank: channel 1 shares a bucket, channel 2 holds its own fluid =="
ctl command "execute in minecraft:overworld run setblock 0 100 8 $TANK"
ctl command "execute in minecraft:overworld run setblock 4 100 8 $TANK"
ctl command "execute in minecraft:overworld run setblock 8 100 8 $TANK"
ctl command "execute in minecraft:overworld run setblock 20 100 8 $TANK"
ctl wait 6
# The tank at 8,100,8 moves to channel 2, and 20,100,8 to channel 3, which is never filled.
stand 8 100 8
say "dimstorage channel 8 100 8 2"
ctl wait 4
stand 20 100 8
say "dimstorage channel 20 100 8 3"
ctl wait 6
expect_tank 0 100 8 "public / 1 (0 / 16000 mB, empty)"
expect_tank 8 100 8 "public / 2 (0 / 16000 mB, empty)"
# A bucket is unstackable, so the harness puts one in a fixed hotbar slot to hold.
ctl command "item replace entity @s hotbar.0 with minecraft:water_bucket 1"
ctl command "item replace entity @s hotbar.1 with minecraft:lava_bucket 1"
ctl wait 6
stand 0 100 8
use_on minecraft:water_bucket 0 100 8
expect_tank 0 100 8 "public / 1 (1000 / 16000 mB of minecraft:water)"
if [ "$SELFTEST" = tank ]; then
    # Deliberately wrong: the sibling tank of channel 1 holds the same fluid.
    echo "(selftest: asserting the shared tank is empty)"
    expect_tank 4 100 8 "(0 / 16000 mB, empty)"
else
    expect_tank 4 100 8 "public / 1 (1000 / 16000 mB of minecraft:water)"
fi
expect_tank 8 100 8 "(0 / 16000 mB, empty)"
expect_tank 20 100 8 "(0 / 16000 mB, empty)"
echo "  channel 1 shared the bucket; channel 2 and the unused channel 3 stayed empty"

echo "== an empty bucket drains the channel; a second fluid stays on its channel =="
stand 4 100 8
use_on minecraft:bucket 4 100 8
expect_tank 0 100 8 "public / 1 (0 / 16000 mB, empty)"
expect_tank 4 100 8 "public / 1 (0 / 16000 mB, empty)"
water_buckets=$(inv_count minecraft:water_bucket)
echo "  water buckets after the drain: $water_buckets (the drained bucket came back full)"
[ "$water_buckets" = 1 ] || fail "the drain left $water_buckets water buckets, expected 1"
stand 8 100 8
use_on minecraft:lava_bucket 8 100 8
expect_tank 8 100 8 "public / 2 (1000 / 16000 mB of minecraft:lava)"
expect_tank 0 100 8 "(0 / 16000 mB, empty)"
echo "  a lava bucket filled only the channel-2 tank"

echo "== a tank channel caps at sixteen buckets =="
# A bucket is unstackable, so the harness refills one hotbar slot before each fill.
held_before_cap=$(inv_count minecraft:water_bucket)
stand 0 100 8
for _ in $(seq 1 16); do
    # The harness player drifts out of reach over many cycles, so it is put back.
    stand 0 100 8
    ctl command "item replace entity @s hotbar.0 with minecraft:water_bucket 1" >/dev/null \
        || fail "cannot hand the player a water bucket"
    ctl wait 3
    use_on minecraft:water_bucket 0 100 8
done
expect_tank 0 100 8 "public / 1 (16000 / 16000 mB of minecraft:water)"
echo "  water buckets left after 16 fills: $(inv_count minecraft:water_bucket)"
# The 17th bucket does not fit: the interaction is refused before anything moves,
# the tank stays at its cap and the bucket is not emptied. The tank's own saved
# compound is read too, so the refusal is proven on both sides of the capability.
ctl command "item replace entity @s hotbar.0 with minecraft:water_bucket 1"
ctl wait 4
stand 0 100 8
before17=$(inv_count minecraft:water_bucket)
compound_before=$(tank_compound 0 100 8)
echo "  block compound before the 17th fill: $compound_before"
use_on minecraft:water_bucket 0 100 8
after17=$(inv_count minecraft:water_bucket)
compound_after=$(tank_compound 0 100 8)
echo "  water buckets around the refused 17th fill: $before17 -> $after17"
[ "$after17" = "$before17" ] || fail "the 17th bucket was emptied into a full tank ($before17 -> $after17)"
[ "$compound_after" = "$compound_before" ] \
    || fail "the refused 17th fill changed the tank's own compound: '$compound_before' -> '$compound_after'"
case "$compound_after" in
    *Amount:16000*minecraft:water*|*minecraft:water*Amount:16000*) ;;
    *) fail "the full tank's own compound reads '$compound_after', not 16000 of water" ;;
esac
if [ "$SELFTEST" = capacity ]; then
    # Deliberately wrong: a refused fill does not raise the channel past its cap.
    echo "(selftest: asserting the channel took the refused 17th bucket)"
    expect_tank 0 100 8 "public / 1 (17000 / 16000 mB of minecraft:water)"
else
    expect_tank 0 100 8 "public / 1 (16000 / 16000 mB of minecraft:water)"
fi

echo "== a drain is committed at access; a stale sibling cannot restore it =="
# Freeze the clock so no block entity ticks between draining a bucket through tank
# A (0,100,8) and removing sibling B (4,100,8). B's saved compound then still says
# 16,000 while the channel has lost the bucket — the restart the review names.
# The drain must be in the channel before the click reports success.
ctl command "tick freeze" >/dev/null
sleep 1
stand 0 100 8
ctl command "item replace entity @s hotbar.0 with minecraft:bucket 1" >/dev/null
sleep 1
ctl select minecraft:bucket >/dev/null
ctl interact 0 100 8 >/dev/null || fail "using the empty bucket on the tank failed"
sleep 2
ctl command "execute in minecraft:overworld run setblock 4 100 8 minecraft:air" >/dev/null
sleep 1
frozen_buckets=$(inv_count minecraft:water_bucket)
echo "  water buckets after the frozen drain: $frozen_buckets"
[ "$frozen_buckets" = 1 ] || fail "the frozen drain gave $frozen_buckets water buckets, expected 1"
if [ "$SELFTEST" = transfer ]; then
    # Deliberately wrong: the drain is committed, so the channel holds 15,000.
    drained=16000
else
    drained=15000
fi
frozen_compound=$(tank_compound 0 100 8)
echo "  block compound before the relaunch: $frozen_compound"
case "$frozen_compound" in
    *"Amount:$drained"*minecraft:water*|*minecraft:water*"Amount:$drained"*) ;;
    *) fail "the drained tank's own compound reads '$frozen_compound', not $drained of water" ;;
esac
ctl save >/dev/null
ctl quit >/dev/null 2>&1 || true
sleep 3
launch
expect_tank 0 100 8 "public / 1 ($drained / 16000 mB of minecraft:water)"
compound=$(tank_compound 0 100 8)
echo "  block compound after the relaunch: $compound"
case "$compound" in
    *"Amount:$drained"*minecraft:water*|*minecraft:water*"Amount:$drained"*) ;;
    *) fail "after the relaunch the drained tank's own compound reads '$compound', not $drained of water" ;;
esac
[ "$(ctl block 4 100 8 | jq -r '.block // .name // empty')" = "minecraft:air" ] \
    || fail "the sibling tank at 4,100,8 was not removed"
echo "  the channel stayed at $drained mB after removing the stale sibling and relaunching"

has 0 100 0 minecraft:diamond 7
echo "== breaking a tank drops it (survival) and leaves the store intact =="
# A creative break drops nothing, so this leg plays in survival with a pickaxe,
# over a hopper that catches the drop. A chest gone through a hopper would hand
# the channel's diamonds down instead (a hopper pulls from a container above it),
# which is why the chest's table is rolled directly with /loot instead.
ctl command "execute in minecraft:overworld run setblock 2 99 12 minecraft:hopper"
ctl wait 6
ctl give minecraft:iron_pickaxe 1
ctl wait 4
ctl select minecraft:iron_pickaxe
ctl wait 4
ctl command "gamemode survival @s"
ctl wait 4
stand 2 100 12
ctl break 2 100 12 >/dev/null || fail "cannot break the tank at 2,100,12"
ctl wait 40
ctl command "gamemode creative @s"
ctl wait 4
has 2 99 12 "$TANK" 1
echo "  hopper(2,99,12)=$(slots 2 99 12)"
expect_tank 0 100 8 "public / 1 (15000 / 16000 mB of minecraft:water)"
has 0 100 0 minecraft:diamond 7
expect_info 0 100 0 "public / 1"
ctl command "execute in minecraft:overworld run setblock 2 100 12 $TANK"
ctl wait 10
expect_tank 2 100 12 "public / 1 (15000 / 16000 mB of minecraft:water)"
echo "  a fresh tank found the channel's water after the break"

echo "== every carried loot table rolls its own item =="
for table_and_item in "dimstorage:blocks/dimensional_chest $BLOCK" "dimstorage:blocks/dimensional_tank $TANK"; do
    set -- $table_and_item
    before=$(inv_count "$2")
    reply=$(ctl command "loot give @s loot $1")
    echo "  $1: $(jq -r '.chat[]?' <<<"$reply")"
    jq -e '.ok == true and (.chat[0] | test("Dropped 1"))' <<<"$reply" >/dev/null \
        || fail "looting $1 answered $reply"
    ctl wait 6
    after=$(inv_count "$2")
    [ "$after" = "$((before + 1))" ] || fail "$1 gave $((after - before)) x $2, expected 1"
    echo "  $1 rolled 1 x $2"
done

echo "== both store files exist and both stores survive a relaunch =="
ctl wait 40
STORE="$SCRATCH/saves/$WORLD/dimstorage/inventories.dat"
TANKS="$SCRATCH/saves/$WORLD/dimstorage/tanks.dat"
[ -s "$STORE" ] || fail "missing store file $STORE"
[ -s "$TANKS" ] || fail "missing tank file $TANKS"
grep -q 'f public,1' "$STORE" || fail "the store file has no public/1 frequency: $(head -20 "$STORE")"
grep -q 'f public,2' "$STORE" || fail "the store file has no public/2 frequency: $(head -20 "$STORE")"
# An empty channel is not retained (0.2.1: visiting or retuning a chest must not materialize a
# permanent 54-slot entry), so channel 3 — which never held anything — is absent on purpose.
if grep -q 'f public,3' "$STORE"; then
    fail "the store kept the empty public/3 channel: $(head -20 "$STORE")"
fi
grep -q 't public,1 minecraft:water 15000' "$TANKS" \
    || fail "the tank file has no public/1 water: $(head -20 "$TANKS")"
grep -q 't public,2 minecraft:lava 1000' "$TANKS" \
    || fail "the tank file has no public/2 lava: $(head -20 "$TANKS")"
if grep -q 't public,3' "$TANKS"; then
    fail "the tank file kept the empty public/3 channel: $(head -20 "$TANKS")"
fi
echo "  inventories: $(grep '^f ' "$STORE" | tr '\n' ' ')"
echo "  tanks: $(grep '^t ' "$TANKS" | tr '\n' ' ')"
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
expect_tank 0 100 8 "public / 1 (15000 / 16000 mB of minecraft:water)"
expect_tank 8 100 8 "public / 2 (1000 / 16000 mB of minecraft:lava)"
expect_tank 20 100 8 "(0 / 16000 mB, empty)"

echo "PASS: the tank recipe crafted and placed, same-channel tanks shared one fluid while another channel held its own, an empty bucket drained the channel, a channel capped at 16000 mB and refused the 17th bucket before anything moved, a frozen-clock drain through one tank was committed at access and a removed stale sibling could not restore it across a relaunch (the channel read exactly 15000 mB in its own compound and in the store), a broken tank dropped into a hopper with its fluid intact, same-frequency chests shared 54 slots, a vanilla barrel never shared, and both stores survived a relaunch"
