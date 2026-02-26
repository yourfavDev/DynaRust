#!/bin/bash

# Configuration
PORT=6660
BASE_URL="http://localhost:$PORT"
TOTAL_RECORDS=10000  # Change to 100000 for the full test
CONCURRENCY=50       # Number of parallel requests
TABLE="test_perf"

echo "=== Starting Performance Test ($TOTAL_RECORDS records) ==="

# 1) Register & Authenticate Alice
echo "[1/4] Registering and authenticating user..."
curl -s -X POST "$BASE_URL/auth/alice" \
  -H "Content-Type: application/json" \
  -d '{"secret":"s3cr3t"}' > /dev/null

TOKEN=$(curl -s -X POST "$BASE_URL/auth/alice" \
  -H "Content-Type: application/json" \
  -d '{"secret":"s3cr3t"}' | jq -r .token)

if [ -z "$TOKEN" ] || [ "$TOKEN" == "null" ]; then
    echo "Error: Failed to retrieve JWT token. Is the server running?"
    exit 1
fi

# 50-word Lorem Ipsum JSON payload
PAYLOAD='{"value": "Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident."}'

# 2) Parallel PUT Operations (Data Storage Test)
echo "[2/4] Inserting $TOTAL_RECORDS records with $CONCURRENCY concurrent workers..."
START_TIME=$(date +%s)

# We use seq and xargs to blast the server with concurrent requests
seq 1 $TOTAL_RECORDS | xargs -n 1 -P $CONCURRENCY -I {} \
  curl -s -X PUT "$BASE_URL/$TABLE/key/req_{}" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d "$PAYLOAD" -o /dev/null

END_TIME=$(date +%s)
echo "Done! Insertion took $((END_TIME - START_TIME)) seconds."

# 3) GET Operation Latency Test
echo "[3/4] Measuring GET Latency..."
TOTAL_TIME=0
SAMPLE_SIZE=100

for i in $(seq 1 $SAMPLE_SIZE); do
    # Fetch a random key from the ones we just created
    RANDOM_KEY="req_$((1 + $RANDOM % $TOTAL_RECORDS))"
    
    # Use curl's write-out feature to get the exact time in seconds, then convert to ms
    TIME=$(curl -s -o /dev/null -w "%{time_total}" "$BASE_URL/$TABLE/key/$RANDOM_KEY")
    TOTAL_TIME=$(awk "BEGIN {print $TOTAL_TIME + $TIME}")
done

AVG_LATENCY=$(awk "BEGIN {print ($TOTAL_TIME / $SAMPLE_SIZE) * 1000}")
echo "Average GET Latency over $SAMPLE_SIZE requests: ${AVG_LATENCY} ms"

# 4) System Stats & Memory
echo "[4/4] Fetching Node Stats..."
curl -s "$BASE_URL/stats" | jq .

echo "=== System Resource Check ==="
# Find the PID of the server running on the port
PID=$(lsof -t -i:$PORT)
if [ -n "$PID" ]; then
    echo "Memory Consumption of Server (PID $PID):"
    ps -p $PID -o %cpu,%mem,rss | awk 'NR==1{print $0; next} {print $1"%  ", $2"%  ", $3/1024 " MB"}'
else
    echo "Could not find PID for port $PORT to check memory."
fi

# Check disk space used by the cold storage (assuming it's in the current directory or a specific data dir)
# Adjust the path './' if your AppState base_dir is located elsewhere
echo "Disk Space Used by storage directories:"
du -sh ./*/ 2>/dev/null | grep -E "auth|test|default|$TABLE" || echo "No data directories found in current path."

echo "=== Test Complete ==="
