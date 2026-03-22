# DynaRust Integration Test Suite Explanation

This document explains the step-by-step logic of the `test.sh` script, which validates the core distributed features of DynaRust.

## Overview
The test suite automates the lifecycle of a 3-node cluster, simulating real-world production events like network partitions, node failures, and massive write bursts.

---

## 1. Environment Initialization
- **Cleanup:** Kills any lingering `DynaRust` processes and wipes the `data/` and `snapshots/` directories to ensure a "Shared Nothing" starting state.
- **Build:** Recompiles the project in `--release` mode to ensure performance-sensitive features (like `DashMap` and `AES-GCM` encryption) are tested at full speed.

## 2. Cluster Formation & SWIM Gossip
- **Dynamic Join:** Node 2 and Node 3 join the cluster via Node 1.
- **SWIM Protocol:** The script waits 15 seconds for the **SWIM-inspired gossip protocol** to propagate membership. It verifies that every node's "view" of the cluster shows 3 active members.

## 3. Identity Sync (Multi-Node Authentication)
- **Registration Replication:** Alice registers on Node 1. The test then attempts to log in as Alice on a *different* node.
- **Verification:** This confirms that the internal `auth` table is being replicated across the cluster, allowing users to authenticate against any entry point.

## 4. Conflict Resolution (Network Partition)
This is the most complex part of the test suite:
1.  **Isolation:** We send a `SIGSTOP` to Node 2, effectively freezing it and simulating a network partition (Split Brain).
2.  **Divergent Updates:** While Node 2 is frozen, we update a key on Node 1.
3.  **Healing:** We resume Node 2 (`SIGCONT`) and immediately perform a conflicting update on it.
4.  **Reconciliation:** The test waits for the nodes to communicate. It verifies that **Vector Clocks** correctly identified the conflict and used **Last-Writer-Wins (LWW)** to bring both nodes to the same final state.

## 5. Massive Concurrency Stress Test
- **Throughput:** Fires **1,000 PUT requests** across 20 parallel threads using `xargs`.
- **DashMap Validation:** Tests if the shard-based locking in `DashMap` handles high-frequency writes without deadlocks or data corruption.
- **Total Replication:** Verifies that after the burst, all 3 nodes report a key count of exactly 1,000.

## 6. Persistence & Hard Cluster Recovery
- **Cold Save:** The test triggers the periodic persistence task which encrypts the in-memory store into chunked `storage.db` files.
- **Nuclear Option:** Kills the entire cluster (simulating a power failure).
- **Restoration:** Restarts a node and verifies that the data is correctly decrypted and reloaded from disk into memory.

## 7. SSE Multiplexing
- **Real-time Broadcast:** Spawns 3 parallel background listeners on the same key.
- **Validation:** Performs a single write and verifies that **all 3 subscribers** received the update event in real-time, confirming the `SubscriptionManager` handles multiplexing correctly.

## 8. Admin API & Ownership Bypass
- **Security Check:** Verifies that a normal user (Bob) cannot overwrite Alice's data.
- **Admin Elevation:** Authenticates as an admin and verifies that the admin *can* overwrite Alice's data, ensuring the "backdoor" management functions work for cluster maintainers.

## 9. Statistics & Health
- **Metrics Middleware:** Validates that the request counter and latency tracking are active and reporting non-zero values.

---

## How to Run
```bash
chmod +x test.sh
./test.sh
```
*Note: Ensure `jq` is installed on your system as it is used to parse JSON responses.*
