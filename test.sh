#!/bin/bash

# DynaRust Production-Level Integration & Stress Test Suite
# Focuses on edge cases: Network partitions, Conflict resolution, 
# Persistence integrity, and Massive Concurrency.

set -e

# Configuration
NODE1_ADDR="127.0.0.1:6661"
NODE2_ADDR="127.0.0.1:6662"
NODE3_ADDR="127.0.0.1:6663"
CLUSTER_SECRET="prod_secret_99"
ADMIN_PASSWORD="super_admin_pass"
JWT_SECRET="ultra_secure_jwt_123"
TABLE="main_prod_table"
STRESS_TABLE="stress_test_table"

# Colors
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

FAILED_TESTS=()

log() { echo -e "${CYAN}[$(date +%T)]${NC} $1"; }
ok() { echo -e "${GREEN}  ✓ $1${NC}"; }
warn() { echo -e "${YELLOW}  ! $1${NC}"; }
error() { echo -e "${RED}  ✗ $1${NC}"; FAILED_TESTS+=("$1"); }

assert_status() {
    local actual=$1
    local expected=$2
    local msg=$3
    if [ "$actual" != "$expected" ]; then
        error "$msg (Expected $expected, got $actual)"
        return 1
    fi
    return 0
}

# 1) Environment Setup
log "Initializing Test Environment..."
export JWT_SECRET=$JWT_SECRET
export CLUSTER_SECRET=$CLUSTER_SECRET
export ADMIN_PASSWORD=$ADMIN_PASSWORD

killall DynaRust 2>/dev/null || true
rm -rf data data_* snapshots *.log 2>/dev/null || true

# 2) Build
log "Compiling DynaRust (Release)..."
cargo build --release > /dev/null 2>&1

# 3) Cluster Launch
log "Starting 3-Node Cluster..."
./target/release/DynaRust "$NODE1_ADDR" > node1.log 2>&1 &
PID1=$!
sleep 1
./target/release/DynaRust "$NODE2_ADDR" "$NODE1_ADDR" > node2.log 2>&1 &
PID2=$!
sleep 1
./target/release/DynaRust "$NODE3_ADDR" "$NODE1_ADDR" > node3.log 2>&1 &
PID3=$!

cleanup() {
    log "Emergency Cleanup..."
    kill $PID1 $PID2 $PID3 2>/dev/null || true
}
trap cleanup EXIT

log "Waiting for SWIM Gossip to stabilize (15s)..."
sleep 15

# 4) Verify Membership
ACTIVE_COUNT=$(curl -s "http://$NODE1_ADDR/membership" | jq -r '.active_count')
if [ "$ACTIVE_COUNT" -eq 3 ]; then
    ok "3 nodes active in cluster"
else
    error "Membership failed (got $ACTIVE_COUNT active nodes)"
fi

# 5) Auth & User Provisioning
log "Testing Authentication & Identity Propagation..."
curl -s -X POST "http://$NODE1_ADDR/auth/alice" -H "Content-Type: application/json" -d '{"secret":"alice_pass"}' > /dev/null
ALICE_TOKEN=$(curl -s -X POST "http://$NODE1_ADDR/auth/alice" -H "Content-Type: application/json" -d '{"secret":"alice_pass"}' | jq -r .token)

curl -s -X POST "http://$NODE3_ADDR/auth/bob" -H "Content-Type: application/json" -d '{"secret":"bob_pass"}' > /dev/null
sleep 2
BOB_TOKEN=$(curl -s -X POST "http://$NODE1_ADDR/auth/bob" -H "Content-Type: application/json" -d '{"secret":"bob_pass"}' | jq -r .token)

if [[ -n "$ALICE_TOKEN" && "$ALICE_TOKEN" != "null" && -n "$BOB_TOKEN" && "$BOB_TOKEN" != "null" ]]; then
    ok "Multi-node Identity Sync successful"
else
    error "Identity Sync Failed"
fi

# 6) Complex Conflict Resolution (Vector Clocks + LWW)
log "Testing Causal Consistency & Conflict Resolution..."
curl -s -X PUT "http://$NODE1_ADDR/$TABLE/key/conflict_key" \
  -H "Authorization: Bearer $ALICE_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"val": "v0", "note": "initial"}' > /dev/null

log "Simulating Network Partition: Isolating Node 2..."
kill -STOP $PID2 

curl -s -X PATCH "http://$NODE1_ADDR/$TABLE/key/conflict_key" \
  -H "Authorization: Bearer $ALICE_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"val": "v1_from_n1"}' > /dev/null

kill -CONT $PID2
curl -s -X PATCH "http://$NODE2_ADDR/$TABLE/key/conflict_key" \
  -H "Authorization: Bearer $ALICE_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"val": "v2_from_n2"}' > /dev/null

log "Partition healed. Verifying eventual convergence..."
sleep 10

VAL1=$(curl -s "http://$NODE1_ADDR/$TABLE/key/conflict_key" -H "Authorization: Bearer $ALICE_TOKEN" | jq -r '.value.val')
VAL2=$(curl -s "http://$NODE2_ADDR/$TABLE/key/conflict_key" -H "Authorization: Bearer $ALICE_TOKEN" | jq -r '.value.val')

if [ "$VAL1" == "$VAL2" ]; then
    ok "Cluster converged to value: $VAL1"
else
    error "Cluster Divergence: Node1=$VAL1, Node2=$VAL2"
fi

# 7) Massive Concurrency Stress Test
log "Executing Massive Concurrency Stress Test (1000 Keys, 20 parallel threads)..."
START_TIME=$(date +%s)
seq 1 1000 | xargs -n 1 -P 20 -I {} \
  curl -s -X PUT "http://$NODE3_ADDR/$STRESS_TABLE/key/stress_{}" \
  -H "Authorization: Bearer $BOB_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"data": "some_payload_{}", "ts": '$(date +%s)'}' > /dev/null
END_TIME=$(date +%s)
DURATION=$(( END_TIME - START_TIME ))
log "Stress test completed in ${DURATION}s"

sleep 5
COUNT1=$(curl -s "http://$NODE1_ADDR/$STRESS_TABLE/keys" | jq '. | length')
COUNT2=$(curl -s "http://$NODE2_ADDR/$STRESS_TABLE/keys" | jq '. | length')
COUNT3=$(curl -s "http://$NODE3_ADDR/$STRESS_TABLE/keys" | jq '. | length')

if [[ "$COUNT1" -eq 1000 && "$COUNT2" -eq 1000 && "$COUNT3" -eq 1000 ]]; then
    ok "All 1000 keys replicated correctly under load"
else
    error "Consistency under stress: N1=$COUNT1, N2=$COUNT2, N3=$COUNT3"
fi

# 8) Persistence & Snapshot Restoration
log "Testing Persistence & Hard Cluster Recovery..."
log "Triggering cold save (Wait 35s)..."
sleep 35

log "Hard Cluster Reset (Killing all nodes)..."
kill $PID1 $PID2 $PID3 2>/dev/null || true
sleep 2

log "Restarting Node 1 from local storage..."
./target/release/DynaRust "$NODE1_ADDR" > node1_recovery.log 2>&1 &
PID1=$!
sleep 5

RECOVERED_VAL=$(curl -s "http://$NODE1_ADDR/$TABLE/key/conflict_key" -H "Authorization: Bearer $ALICE_TOKEN" | jq -r '.value.val')
if [[ "$RECOVERED_VAL" == "v1_from_n1" || "$RECOVERED_VAL" == "v2_from_n2" ]]; then
    ok "Persistence integrity verified: $RECOVERED_VAL"
else
    error "Persistence failure: recovered $RECOVERED_VAL"
fi

# 9) SSE Multiplexing
log "Testing SSE Multiplexing (3 parallel subscribers)..."
SSE_OUT1="sse1.txt"; SSE_OUT2="sse2.txt"; SSE_OUT3="sse3.txt"
rm -f sse*.txt
curl -s --no-buffer "http://$NODE1_ADDR/$TABLE/subscribe/multi_key" > $SSE_OUT1 &
P_SSE1=$!
curl -s --no-buffer "http://$NODE1_ADDR/$TABLE/subscribe/multi_key" > $SSE_OUT2 &
P_SSE2=$!
curl -s --no-buffer "http://$NODE1_ADDR/$TABLE/subscribe/multi_key" > $SSE_OUT3 &
P_SSE3=$!
sleep 2

curl -s -X PUT "http://$NODE1_ADDR/$TABLE/key/multi_key" \
  -H "Authorization: Bearer $ALICE_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"update": "broadcasting"}' > /dev/null
sleep 2

kill $P_SSE1 $P_SSE2 $P_SSE3 2>/dev/null || true
C1=$(grep -c "Updated" $SSE_OUT1 || true); C2=$(grep -c "Updated" $SSE_OUT2 || true); C3=$(grep -c "Updated" $SSE_OUT3 || true)

if [[ $C1 -gt 0 && $C2 -gt 0 && $C3 -gt 0 ]]; then
    ok "SSE Multiplexing successful"
else
    error "SSE Multiplexing failure: C1=$C1, C2=$C2, C3=$C3"
fi

# 10) Admin Massive Cleanup
log "Testing Admin API..."
ADMIN_TOKEN=$(curl -s -X POST "http://$NODE1_ADDR/admin/login" -H "Content-Type: application/json" -d '{"password":"'$ADMIN_PASSWORD'"}' | jq -r .token)
DEL_STATUS=$(curl -s -o /dev/null -w "%{http_code}" -X DELETE "http://$NODE1_ADDR/admin/table/$STRESS_TABLE/key/stress_1" \
  -H "Authorization: Bearer $ADMIN_TOKEN")
assert_status "$DEL_STATUS" "200" "Admin deletion"

# Final Stats
log "Verifying Statistics..."
STATS=$(curl -s "http://$NODE1_ADDR/stats")
ok "Final Request Count: $(echo "$STATS" | jq -r '.total_requests')"

# Summary
echo -e "\n${CYAN}==========================================${NC}"
if [ ${#FAILED_TESTS[@]} -eq 0 ]; then
    echo -e "${GREEN}  ALL COMPLEX PRODUCTION EDGE CASES PASSED! 🚀${NC}"
    echo -e "${CYAN}==========================================${NC}"
    exit 0
else
    echo -e "${RED}  THE FOLLOWING TESTS FAILED:${NC}"
    for f in "${FAILED_TESTS[@]}"; do
        echo -e "${RED}    - $f${NC}"
    done
    echo -e "${CYAN}==========================================${NC}"
    exit 1
fi
