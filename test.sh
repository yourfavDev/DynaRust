#!/bin/bash
# This script:
#  - Clones the DynaRust binary into a unique folder for each node so that each node uses its own file system.
#  - Launches 100 nodes (one seed and 99 joiners) from their own working directories.
#  - Writes a key/value pair to the "default" table on the seed node.
#  - Waits for propagation and performs random reads.
#  - Also tests endpoints for listing keys and retrieving multiple keys.
#  - Finally, it cleans up (kills node processes and removes the temporary directories).

set -e

# Create a temporary directory where we'll create subdirectories for each node.
TMP_DIR=$(mktemp -d -t dynatest.XXXXXX)
echo "Using temporary directory: ${TMP_DIR}"

# Trap exit signals to kill background nodes and cleanup TMP_DIR when the script exits.
cleanup() {
    echo "Cleaning up..."
    kill $(jobs -p) 2>/dev/null || true
    rm -rf "${TMP_DIR}"
}
trap cleanup EXIT

# Total number of cluster nodes to start.
NODES=30
# Base port for the first (seed) node.
BASE_PORT=6660

echo "Starting DynaRust nodes for cluster test..."

# Array to hold the PIDs of launched processes.
PIDS=()

# Start nodes. Each node will be run from its own directory under TMP_DIR.
for (( i=0; i<NODES; i++ )); do
    NODE_DIR="${TMP_DIR}/node${i}"
    mkdir -p "${NODE_DIR}"
    # Copy the binary to the node directory.
    cp ./target/release/DynaRust "${NODE_DIR}/DynaRust"

    PORT=$((BASE_PORT + i))

    if [ $i -eq 0 ]; then
        # Seed node - no join argument.
        echo "Starting seed node at 127.0.0.1:${PORT} in ${NODE_DIR}"
        ( cd "${NODE_DIR}" && ./DynaRust 127.0.0.1:${PORT} ) &
        PIDS+=($!)
    else
        # Other nodes join the seed node.
        echo "Starting node at 127.0.0.1:${PORT} in ${NODE_DIR}, joining 127.0.0.1:${BASE_PORT}"
        ( cd "${NODE_DIR}" && ./DynaRust 127.0.0.1:${PORT} 127.0.0.1:${BASE_PORT} ) &
        PIDS+=($!)
    fi
done

echo "Waiting 5 seconds for cluster stabilization..."
sleep 5

# Define test parameters.
TABLE="default"
KEY="testkey"
VALUE="testvalue"

# Write a test value to the seed node in the "default" table.
echo "Writing key '$KEY' with value '$VALUE' to table '$TABLE' on the seed node..."
curl -s -X PUT "http://127.0.0.1:${BASE_PORT}/${TABLE}/key/${KEY}" \
     -H "Content-Type: application/json" \
     -d "{\"attribute\": \"${VALUE}\"}"
echo ""

echo "Waiting 3 seconds for data propagation..."
sleep 3

# Perform 10 random read operations from nodes in the cluster.
NUM_READS=10
echo "Performing ${NUM_READS} random reads from cluster nodes..."
for (( i=1; i<=NUM_READS; i++ )); do
  NODE_INDEX=$(($RANDOM % $NODES))
  READ_PORT=$((BASE_PORT + NODE_INDEX))
  echo "Read attempt ${i} from node at 127.0.0.1:${READ_PORT}:"
  curl -s "http://127.0.0.1:${READ_PORT}/${TABLE}/key/${KEY}"
  echo -e "\n-------------------------"
done

# Test the new endpoint to list all keys in the table from the seed node.
echo "Listing all keys in table '$TABLE' from the seed node:"
curl -s "http://127.0.0.1:${BASE_PORT}/${TABLE}/keys"
echo -e "\n-------------------------"

# Test the new endpoint to retrieve multiple keys from the table from the seed node.
echo "Retrieving multiple keys from table '$TABLE' from the seed node (requesting 'testkey' and 'nonexistent'):"
curl -s -X POST "http://127.0.0.1:${BASE_PORT}/${TABLE}/keys" \
     -H "Content-Type: application/json" \
     -d '{"keys": ["testkey", "nonexistent"]}'
echo -e "\n-------------------------"

# Select 3 random nodes and for each:
# - Retrieve the test key value using GET /default/key/{key}
# - List all keys using GET /default/keys
echo "Selecting 3 random nodes to verify key access and key listing:"
for (( i=1; i<=3; i++ )); do
  NODE_INDEX=$(($RANDOM % $NODES))
  READ_PORT=$((BASE_PORT + NODE_INDEX))
  echo "From node at 127.0.0.1:${READ_PORT}:"
  echo "Retrieving key '$KEY':"
  curl -s "http://127.0.0.1:${READ_PORT}/${TABLE}/key/${KEY}"
  echo ""
  echo "Listing keys in table '$TABLE':"
  curl -s "http://127.0.0.1:${READ_PORT}/${TABLE}/keys"
  echo -e "\n-------------------------"
done
pkill DynaRust

echo "Test completed successfully."
