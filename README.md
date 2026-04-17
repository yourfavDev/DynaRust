# 🦀 DynaRust: Distributed Key-Value Store

DynaRust is a high-performance, distributed key-value store built in Rust. Designed for massive concurrency, reliable consistency, and seamless scalability, it leverages lock-free concurrent storage, robust binary internal replication, causal consistency via Vector Clocks, and a SWIM-inspired gossip protocol to deliver a fault‑tolerant, horizontally scalable datastore.

Optimized for modern hardware, a single node can sustain peak traffic of up to **10,000+ live connections** with sub-5 ms latency for real-time updates. Cluster capacity scales linearly by adding more nodes.

## 📊 Performance (v2)

| Metric | Implementation Details |
| :--- | :--- |
| **Storage Engine** | DashMap (Granular Shard-based Locking) |
| **Internal Protocol** | Bincode (Fast, Compact Binary Serialization) |
| **Consistency Model** | Eventual Consistency via Vector Clocks |
| **Replication Strategy**| Consistent Hashing Ring with Virtual Nodes |
| **Reliability** | Read Repair + Exponential Backoff Retries |
| **Gossip Protocol** | SWIM-inspired ($O(1)$ Scalable Membership) |


## 📸 Cluster in Action

**Main Node Running:**
![main](https://github.com/yourfavDev/DynaRust/blob/42013f18f1f4d0ede1cad81ed1249e42f12f2951/docs/main.png)
## ⏎ Second node joins the main node (forming a cluster)
![second](https://github.com/yourfavDev/DynaRust/blob/42013f18f1f4d0ede1cad81ed1249e42f12f2951/docs/second.png)

-----

## ✨ Key Features

  * **Massive Concurrency:** Utilizes granular, shard-based locking (via `DashMap`) instead of global `RwLock`s, preventing bottlenecks during simultaneous read/write operations across different tables and keys.
  * **Causal Consistency:** Employs Vector Clocks for version tracking, accurately monitoring causality across distributed nodes and automatically resolving concurrent updates to ensure data integrity.
  * **Intelligent Replication & Reliability:**
      * **Consistent Hashing:** Minimizes data remapping when nodes dynamically join or leave the cluster.
      * **Read Repair:** `GET` requests check replicas; stale versions are automatically repaired in the background.
      * **Binary Replication:** Node-to-node communication uses Bincode, reducing CPU usage and network saturation.
  * **Scalable Membership:** Network overhead for health checks remains constant regardless of cluster size via an indirect-probing SWIM gossip protocol.
  * **Real-Time Updates:** Clients can subscribe to keys via Server-Sent Events (SSE) to receive instant change notifications.
  * **High Availability & Persistence:** Automated failover routing, periodic JSON snapshots, and Write-Ahead Logging (WAL) to a local `storage.db` ensure data durability.

-----

## 🔒 Security Architecture

Security is enforced from user access down to node-to-node transport:

1.  **Access Control:** All `PUT`, `PATCH`, and `DELETE` operations require a JWT Bearer token. Only the verified owner of a record can modify or delete it.
2.  **Cluster Authentication:** \* Each node must have a `JWT_SECRET` configured to start.
      * Nodes must present a shared secret (`CLUSTER_SECRET` environment variable) to join the cluster.
      * A SHA256 encryption key is embedded at compile time (via `bash encryption.sh && cargo build --release`).
3.  **Transport Security:** Native HTTPS mode is supported. Run `bash cert.sh` to generate a `.p12` certificate and set `DYNA_MODE=https`.

-----

## 🧠 Architecture Overview

DynaRust guarantees eventual consistency via asynchronous state synchronization. Requests are processed locally in memory, persisted to disk, and fanned out to peer nodes.

```ascii
                    +----------------+
                    |    Client      |
                    +----------------+
                        │ │   │ │
     HTTP: POST /auth   │ │   │ │ HTTP: PUT/GET/DELETE /{table}/key/{key}
     (register/login)   │ │   │ │
                        ↓ ↓   ↓ ↓
               +-----------------------+
               |  API Gateway (Actix)  |
               |  • Auth & Validation  | ← issues/verifies JWT
               |  • Route Handling     |
               |  • SSE Subscriptions  | 
               +-----------------------+
                          │
                          │ local reads/writes
                          ↓
               +-----------------------+
               |  In‐Memory Store      |
               |  (DashMap)            |
               +-----------------------+
                  │           │
      internal    │           │ external
      writes  ←───┘           │ writes
                             ↓
               +-----------------------+
               |  Replication Module   |
               |  (Fan‐out to peers)   |
               +-----------------------+
                  │           │
                  │ gossip    │ HTTP
                  │           ↓
               +------------------------------------+
               |     Other Nodes (Replicas)         |
               +------------------------------------+
                          │
                          │ periodic snapshot & WAL
                          ↓
               +-----------------------+
               |  Disk Persistence     |
               |  (storage.db)         |
               +-----------------------+
```

-----

## 🚀 Getting Started

### Option 1: Build from Source

1.  **Clone the Repository:**
    ```bash
    git clone https://github.com/yourfavDev/DynaRust
    cd DynaRust
    ```
2.  **Compile the Binary:**
    ```bash
    cargo build --release
    ```

### Option 2: Docker Deployment

1.  **Build the Image:**
    ```bash
    docker build -t dynarust:latest .
    docker network create dynanet
    ```
2.  **Run the Main Node:**
    ```bash
    docker run -d --name dynarust-node1 --network dynanet -p 6660:6660 \
      -v dynarust-data1:/app \
      dynarust:latest 0.0.0.0:6660
    ```
3.  **Run Additional Nodes (Cluster Setup):**
    ```bash
    docker run -d --name dynarust-node2 --network dynanet -p 6661:6660 \
      -v dynarust-data2:/app \
      dynarust:latest 0.0.0.0:6660 dynarust-node1:6660
    ```

-----

## 📡 API Reference

All write operations (`PUT`, `PATCH`, `DELETE`) require a valid JWT in the `Authorization: Bearer <token>` header. 

| Operation | Endpoint | Method | Description |
| :--- | :--- | :--- | :--- |
| **Auth** | `/auth/{user}` | `POST` | Register or login (returns JWT token). Requires `{"secret": "pwd"}`. |
| **Create/Update** | `/{table}/key/{key}` | `PUT` | Store a JSON value. Requires Auth. |
| **Partial Update**| `/{table}/key/{key}` | `PATCH`| Merge updates into an existing value. Requires Auth. |
| **Read** | `/{table}/key/{key}` | `GET` | Fetch the latest version of a key. |
| **Delete** | `/{table}/key/{key}` | `DELETE`| Remove a key (owner only). Requires Auth. |
| **List Keys** | `/{table}/keys` | `GET` | Returns an array of all keys in a table. |
| **Batch Fetch** | `/{table}/keys` | `POST` | Pass an array of keys in the body to fetch multiple values. |
| **Fetch Table** | `/{table}/store` | `GET` | Returns the entire key-value store for a specific table. |
| **Subscribe** | `/{table}/subscribe/{key}`| `GET` | Establish an SSE connection for real-time `< 5ms` updates. |
| **Node Stats** | `/stats` | `GET` | View cluster health and node statistics. |

### Quick `curl` Examples

```bash
# 1. Register & get token
TOKEN=$(curl -s -X POST http://localhost:6660/auth/alice \
  -H "Content-Type: application/json" \
  -d '{"secret":"s3cr3t"}' | jq -r .token)

# 2. Store a value
curl -i -X PUT http://localhost:6660/default/key/foo \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d '{"value": {"name": "bar"}}'

# 3. Partially update the value
curl -i -X PATCH http://localhost:6660/default/key/foo \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d '{"age": 30}'

# 4. Subscribe to live changes
curl -N http://localhost:6660/default/subscribe/foo
```

-----

## 🛠️ Troubleshooting

  * **Node Fails to Join:** Ensure the target `JOIN_ADDRESS` is reachable. In Docker, confirm the containers are on the same bridge network and you are using the correct internal hostnames/ports.
  * **Inconsistent Reads:** Allow a brief moment for the gossip protocol to propagate state (eventual consistency). You can verify cluster membership by checking `http://<node-address>/stats`.
  * **Data Lost After Restart:** Ensure the process has write permissions to the directory containing `storage.db`. If using Docker, verify that you have properly mounted a volume to `/app`.
  * **Debugging:** Enable verbose logging by running the node with `RUST_LOG=debug`.

-----