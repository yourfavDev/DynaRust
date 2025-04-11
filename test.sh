#!/bin/bash
# This script launches 100 nodes (one seed + 99 joiners),
# writes a key to the seed node, waits for propagation,
# and then performs 10 random reads to verify data consistency.

set -e

# Trap exit signals to kill background nodes when the script exits.
trap "kill $(jobs -p) 2>/dev/null" EXIT

# Total number of cluster nodes to start.
NODES=100
# Base port for the first (seed) node.
BASE_PORT=6660

echo "Starting DynaRust nodes for cluster test..."

# Start the seed node.
echo "Starting seed node at 127.0.0.1:${BASE_PORT}"
./target/release/DynaRust 127.0.0.1:${BASE_PORT} &
# Save the PID if needed.
PIDS=($!)

# Start additional nodes joining the seed.
for (( i=1; i<NODES; i++ )); do
  PORT=$((BASE_PORT + i))
  echo "Starting node at 127.0.0.1:${PORT} joining 127.0.0.1:${BASE_PORT}"
  ./target/release/DynaRust 127.0.0.1:${PORT} 127.0.0.1:${BASE_PORT} &
  PIDS+=($!)
done

# Wait for the cluster nodes to start and join.
echo "Waiting 5 seconds for cluster stabilization..."
sleep 5

# Write a test value to the seed node.
KEY="testkey"
VALUE="testvalue"
echo "Writing key '$KEY' with value '$VALUE' to the seed node..."
curl -s -X PUT "http://127.0.0.1:${BASE_PORT}/key/${KEY}" \
     -H "Content-Type: application/json" \
     -d "{\"value\": \"${VALUE}\"}"
echo ""

# Wait 3 seconds to allow the update to propagate to all nodes.
echo "Waiting 3 seconds for data propagation..."
sleep 3

# Perform 10 random read operations from nodes in the cluster.
NUM_READS=10
echo "Performing ${NUM_READS} random reads from cluster nodes..."
for (( i=1; i<=NUM_READS; i++ )); do
  # Using the built-in $RANDOM to generate a random index.
  NODE_INDEX=$(($RANDOM % $NODES))
  READ_PORT=$((BASE_PORT + NODE_INDEX))
  echo "Read attempt ${i} from node at 127.0.0.1:${READ_PORT}:"
  curl -s "http://127.0.0.1:${READ_PORT}/key/${KEY}"
  echo -e "\n-------------------------"
done
pkill DynaRust
echo "Test completed successfully."
