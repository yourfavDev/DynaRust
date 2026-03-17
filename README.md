# 🦀 DynaRust: Distributed Key-Value Store

DynaRust is a distributed key–JSON‑value built in Rust. It's designed to be reliable and easy to manage, allowing you to add or remove nodes (servers) dynamically without interrupting service 🔄.

It combines in‑memory caching, on‑disk persistence, automatic cross‑node replication and background synchronization for eventual consistency, delivering a fault‑tolerant, horizontally scalable datastore.

With its advanced real‑time update capabilities, DynaRust pushes live changes with latencies below 5 ms 🚀. In fact, on a typical VPS (1 GB RAM, 100 Mbps bandwidth), a single node can comfortably sustain peak traffic of up to **5000 live connections** 🔥—and you can increase capacity even further simply by adding more nodes to your cluster!

---
## Performance
| **Metric**             | **Value**                                     |
|------------------------|-----------------------------------------------|
| Container Resources    | 0.25 vCPU, 0.5 GB RAM                           |
| Data Storage           | 100,000 records (50-word lorem ipsum JSON/record)      |
| Memory Consumption     | 350 MB                                        |
| Cluster Startup        | < 1 sec                                       |
| GET Operation Latency  | ~20 ms                                        |
| Cold storage Usage     | ~30 MB of disk data |

### While inserting ~300 rows/sec we had a SSE client open on a key flawlessly getting live updates in <5 ms (Cheapest AWS EC2)
---

## 🛜 Main node running:
![main](https://github.com/yourfavDev/DynaRust/blob/42013f18f1f4d0ede1cad81ed1249e42f12f2951/docs/main.png)
## ⏎ Second node joins the main node (forming a cluster)
![second](https://github.com/yourfavDev/DynaRust/blob/42013f18f1f4d0ede1cad81ed1249e42f12f2951/docs/second.png)

## ✨ Key Features

*   **🔥 HOT RELOAD & REAL‑TIME UPDATES:**
    Enjoy lightning‑fast, real‑time updates using Server‑Sent Events (SSE). Subscribe to a key with:
    ```bash
    # Example: Subscribe to 'statusKey' in the 'notifications' table
    curl -N http://localhost:8080/notifications/subscribe/statusKey
    ```
    ![liveupdate](https://github.com/yourfavDev/DynaRust/blob/edc84068e9f88be693cdeb7085b19a648ea33b7d/docs/liveupdate.gif)
    Changes are pushed instantly (< 5 ms latency). On a standard VPS (1GB RAM, 100Mbps), a single node handles up to **5000 simultaneous live connections** 💪. Need more capacity? Just add more nodes!

    *   **Use Case Example:** Imagine a web UI needing push notifications. Store device IDs as keys in a `devices` table. Use a separate `status` key in the same table. The frontend listens to `devices/subscribe/status`. The backend iterates through device keys, performs actions, and updates the `status` key, instantly notifying all listening frontends. Simple and blazing fast! ⚡️
---

### 🔒 **Security**

- **Access Control:**
    - **Read:** Only the owner can read the record (passed via bearer header token)
    - **Write/Delete:** Only the record’s owner (as specified in the `owner` field) can modify or delete it.
    - **Enforcement:** All `PUT` and `DELETE` operations require an `Authorization` header. The server verifies that the requester matches the record’s owner.

- **Cluster Security:**
  - At compile time a SHA256 encryption key is embeded in the compiled binary (if that changes somehow in the future (recompile binary with different key) you won't be able to load the table, steps to properly compile are: run bash encryption.sh && cargo build --release and distribute only the binary under target/release/ to your nodes);
    - Each node should have a JWT_SECRET set, without this env var the node won't even start
        - Each node must present a **secret token** (set via the `CLUSTER_SECRET` environment variable) to join the cluster, ensuring only trusted nodes participate.

- **Transport Security (HTTPS):**
    - **Easy Certificate Generation:**
        - Run `bash cert.sh`, provide a password, and a `.p12` certificate will be generated under the `cert/` directory.
    - **HTTPS Mode:**
        - Set `DYNA_MODE=https` to enable HTTPS

---

**_Security is enforced at every layer: from user access to node-to-node communication, ensuring your data remains private and protected._**
*   **🌐 Distributed Storage:**
    Data is automatically partitioned and spread across all nodes in the cluster.

*   **🗄️ Automatic Snapshots:**
    Every 60 minutes DynaRust writes a JSON snapshot of the entire in‑memory store to `./snapshots/snapshot_<ts>.json`.
    By default only the last 100 snapshots are kept; older files are pruned automatically.
    You can override the retention limit with the `SNAP_LIMIT` environment variable (e.g. `SNAP_LIMIT=200`).

*   **✅ High Availability:**
    If one node fails, the remaining nodes continue to serve requests for the available data.

*   **🔄 Dynamic Cluster Membership:**
    Nodes can join or leave the cluster seamlessly without manual re‑configuration.

*   **🤝 Automatic State Sync:**
    New or returning nodes fetch the latest state from the cluster automatically.

*   **💾 Persistent Storage:**
    Data is saved to a local `storage.db` file to prevent data loss upon node restarts.

*   **🔌 RESTful API:**
    A simple HTTP interface (using `GET`, `PUT`, `DELETE`) makes it easy to interact with your data.

---
### Updated API Endpoints (v2)

All operations except **GET** require a valid JWT in the `Authorization: Bearer <token>` header.

1. **🛂 Register / Log In (HTTP POST)**  
   Register a new user or log in an existing one.  
   - URL: `/auth/{user}`  
   - Body:  
     ```json
     { "secret": "my_password" }
     ```  
   - Responses:  
     • `200 OK` and  
       - On first call (user didn’t exist):  
         ```json
         { "status": "User created" }
         ```  
       - On subsequent calls with correct secret:  
         ```json
         { "token": "<JWT‑TOKEN‑HERE>" }
         ```  
     • `400 Bad Request` if user exists on register  
     • `401 Unauthorized` if secret is wrong  

2. **✍️ Store or Update a Value (HTTP PUT)**  
   Create or update a value under `{table}/{key}`.  
   - URL: `/{table}/key/{key}`  
   - Headers:  
     ```
     Content-Type: application/json  
     Authorization: Bearer <JWT‑TOKEN>
     ```  
   - Body:  
     ```json
     { "value": { ... } }
     ```  
   - Success:  
     • `201 Created`  
     • Body (VersionedValue):  
       ```json
       {
         "value": { ... },
         "version": 1,
         "timestamp": 1618880821123,
         "owner": "alice"
       }
       ```  
   - Errors:  
     • `401 Unauthorized` if missing/invalid JWT or not owner on update  

3. **🛠️ Partially Update a Value (HTTP PATCH)**  
   Merge updates into an existing value under `{table}/{key}`. Only the owner can patch.
   - URL: `/{table}/key/{key}`  
   - Headers:  
     ```
     Content-Type: application/json  
     Authorization: Bearer <JWT‑TOKEN>
     ```  
   - Body:  
     ```json
     { "new_field": "updated_data" }
     ```  
   - Success:  
     • `200 OK`  
     • Body (Updated VersionedValue):  
       ```json
       {
         "value": { "original_field": "...", "new_field": "updated_data" },
         "version": 2,
         "timestamp": 1618880825000,
         "owner": "alice"
       }
       ```  
   - Errors:  
     • `401 Unauthorized` if missing/invalid JWT or not owner  
     • `404 Not Found` if key/table missing  

4. **🔍 Retrieve a Value (HTTP GET)**  
   Anyone can fetch a key’s latest value.  
   - URL: `/{table}/key/{key}`  
   - Success:  
     • `200 OK`  
     • Body:  
       ```json
       {
         "value": { ... },
         "version": 1,
         "timestamp": 1618880821123,
         "owner": "alice"
       }
       ```  
     • `404 Not Found` if key/table missing  

5. **🗑️ Delete a Value (HTTP DELETE)**  
   Only the owner may delete.  
   - URL: `/{table}/key/{key}`  
   - Header:  
     ```
     Authorization: Bearer <JWT‑TOKEN>
     ```  
   - Success:  
     • `200 OK`  
     • Body:  
       ```json
       { "message": "Deleted locally" }
       ```  
   - Errors:  
     • `401 Unauthorized` if no JWT or not owner  
     • `404 Not Found` if key/table missing  

6. **📚 Fetch Entire Table Store (HTTP GET)**  
   List all key→VersionedValue pairs in a table.  
   - URL: `/{table}/store`  
   - Success:  
     • `200 OK`  
     • Body:  
       ```json
       {
         "key1": { "value":{...},"version":2,…,"owner":"bob" },
         "key2": { … }
       }
       ```  
   - `404 Not Found` if table missing  

7. **🔑 List or Batch‑Fetch Keys**  
   7.1 **GET** `/{table}/keys`  
       • `200 OK` →  
         ```json
         ["key1","key2",…]
         ```  
   7.2 **POST** `/{table}/keys`  
       - Body:  
         ```json
         ["key1","key2","key3"]
         ```  
       - `200 OK` →  
         ```json
         {
           "key1": { "value":{...},"version":… },
           "key3": { … }
         }
         ```  
       (non‑existent keys are omitted)

8. **🔔 Subscribe to Real‑Time Updates (SSE)**  
   Instant updates on a single key.  
   - URL: `/{table}/subscribe/{key}`  
   - Usage:  
     ```bash
     curl -N http://localhost:6660/{table}/subscribe/{key}
     ```  
   - Each update is a JSON event:  
     ```json
     { "event": "Updated", "value": { … } }
     ```

---

#### Quick `curl` Examples

```bash
# 1) Register Alice
curl -i -X POST http://localhost:6660/auth/alice \
  -H "Content-Type: application/json" \
  -d '{"secret":"s3cr3t"}'

# 2) Log in (get token)
TOKEN=$(curl -s -X POST http://localhost:6660/auth/alice \
  -H "Content-Type: application/json" \
  -d '{"secret":"s3cr3t"}' | jq -r .token)

# 3) PUT (must include token)
curl -i -X PUT http://localhost:6660/default/key/foo \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d '{"value": {"name": "bar"}}'

# 4) PATCH (partially update)
curl -i -X PATCH http://localhost:6660/default/key/foo \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d '{"age": 30}'

# 5) GET
curl -i http://localhost:6660/default/key/foo

# 6) DELETE (owner only)
curl -i -X DELETE http://localhost:6660/default/key/foo \
  -H "Authorization: Bearer $TOKEN"

# 7) Node stats
curl http://localhost:6660/stats
```

> ⚠️ PUT/PATCH/DELETE without a valid JWT → **401 Unauthorized**  
> 🔍 GET is always open (no auth needed).
---

## 🧠 How it Works (Conceptual Overview)

DynaRust uses eventual consistency via state synchronization:

1.  **👤 Client Request:** A client sends an HTTP request (e.g., `PUT /default/key/mykey`) to any node.
2.  **🖥️ Local Processing:** The receiving node updates/reads its **in‑memory** store 💭 immediately.
3.  **💾 Persistence:** The node periodically saves its memory state to `storage.db` for durability.
4.  **🔄 Cluster Synchronization (Gossip):**
    *   Nodes exchange membership info & state updates periodically.
    *   This ensures all nodes eventually converge to the same state (Eventual Consistency).
    *   *Real‑time Updates:* Subscribed clients receive changes via SSE instantly (< 5ms) ⚡️.
    *   **🚀 Performance Highlight:** A typical VPS node (1GB RAM, 100Mbps) handles up to **5000 concurrent live SSE connections**. Add more nodes to scale capacity!
5.  **🤝 Joining:** A new node contacts an existing node (`JOIN_ADDRESS`), fetches the cluster state, and joins.

```ascii
                    +----------------+
                    |    Client      |
                    |     👤         |
                    +----------------+
                        │ │   │ │
     HTTP: POST /auth   │ │   │ │ HTTP: PUT/GET/DELETE /{table}/key/{key}
     (register/login)   │ │   │ │
                        ↓ ↓   ↓ ↓
               +-----------------------+
               |  API Gateway (Actix)  |
               |  • /auth/{user}       | ←── issues JWT on login
               |    - registration      |
               |    - login → JWT      |
               |  • KV endpoints       |
               |    - JWT guard on PUT/DELETE
               |    - GET open to all   |
               |  • /{table}/store,    |
               |    /{table}/keys      |
               |  • /{table}/subscribe │ ←── SSE subscription
               +-----------------------+
                          │
                          │ local reads/writes
                          ↓
               +-----------------------+
               |  In‐Memory Store      |
               |  (HashMap<String,     |
               |   HashMap<String,     |
               |   VersionedValue>)    |
               |         💭            |
               +-----------------------+
                  │           │
      internal    │           │ external
      writes  ←───┘           │   writes
      (X-Internal-Request)    │
                             ↓
               +-----------------------+
               |  Replication Module   |
               |  (Fan‐out PUT/DEL to  |
               |   all other nodes via |
               |   HTTP + X-Internal)  |
               +-----------------------+
                  │           │
                  │ gossip    │ HTTP
                  │           ↓
               +------------------------------------+
               |     Other Nodes (Replicas)         |
               |     — apply internal writes —      |
               |                                    |
               +------------------------------------+
                          │
                          │ periodic
                          │ snapshot & WAL
                          ↓
               +-----------------------+
               |  Disk Persistence     |
               |  (cold_save, WAL)     |
               |         💾            |
               +-----------------------+

Cluster Membership & Gossip (Eventual Consistency)
──────────────────────────────────────────────────
    ┌───────────────┐        gossip(UDP/HTTP)      ┌───────────────┐
    │ Current Node  │  ── heartbeat & membership ─│ Other Node    │
    │  (ClusterData)│  ──────────────────────────>│  (ClusterData)│
    └───────────────┘  <─────────────────────────┘───────────────┘
           🔄                                            🔄

Legend:
 • Client: issues HTTP requests (and SSE connects).
 • API Gateway: Actix routes, JWT auth, validation, SubscriptionManager.
 • In‐Memory Store: local hashmaps of VersionedValue {value,version,timestamp,owner}.
 • Replication Module: fan‑out writes to peers using `X-Internal-Request`.
 • Other Nodes: receive internal requests, update memstore (no auth, no events).
 • Disk Persistence: periodic snapshots + WAL for durability.
 • Cluster Membership: heartbeat sync via broadcaster tasks for node discovery.
 • SSE Subscriptions: real‐time `EventSource` streams on `/subscribe/{key}`.

```

---

## 🐳 Deployment with Docker

Docker simplifies deployment and dependency management.

### 🔧 Prerequisites

*   **Docker:** Version 20.10 or newer installed and running.
*   **🌐 Network Connectivity:** Ensure containers on the same Docker network can reach each other on the `LISTEN_ADDRESS` port (e.g., `6660`).

### ▶️ Steps

1.  **🧱 Build the Image (if not done):**
    ```bash
    docker build -t dynarust:latest .
    ```

2.  **🏃 Run the Container(s):**

    *   **Running the First Node:**
        ```bash
        # Create a network first (recommended)
        docker network create dynanet

        # Start the first node, detached (-d), named, on the network, mapping host port 6660
        docker run -d --name dynarust-node1 --network dynanet -p 6660:6660 \
          dynarust:latest 0.0.0.0:6660
        ```
        *Listens on port 6660 inside the container.*

    *   **Running Additional Nodes:**
        Use the container name (`dynarust-node1`) and internal port (`6660`) as the `JOIN_ADDRESS`.
        ```bash
        # Start a second node, map host port 6661, join node1
        docker run -d --name dynarust-node2 --network dynanet -p 6661:6660 \
          dynarust:latest 0.0.0.0:6660 dynarust-node1:6660
        ```

    *   **💾 Data Persistence (Important!):**
        Mount a volume to persist the `storage.db` file outside the container.
        ```bash
        # Create a named volume (e.g., dynarust-data1)
        docker volume create dynarust-data1

        # Run node1 with the volume mounted to /app (where storage.db is saved)
        docker run -d --name dynarust-node1 --network dynanet -p 6660:6660 \
          -v dynarust-data1:/app \
          dynarust:latest 0.0.0.0:6660

        # Run subsequent nodes with their own volumes
        docker volume create dynarust-data2
        docker run -d --name dynarust-node2 --network dynanet -p 6661:6660 \
          -v dynarust-data2:/app \
          dynarust:latest 0.0.0.0:6660 dynarust-node1:6660
        ```
        *(Adjust the mount source/target `/app` if your Dockerfile places `storage.db` elsewhere).*

---
### 📦 Installation

You can either build DynaRust directly from source or use a pre‑built Docker image.

#### Option 1: Build from Source

1.  **📂 Clone the Repository:**
    ```bash
    git clone https://github.com/yourfavDev/DynaRust # Replace with actual URL if needed
    cd DynaRust
    ```

2.  **🧱 Build the Project:**
    Compile the Rust code for an optimized release build:
    ```bash
    cargo build --release
    ```
    The final binary is located at `target/release/DynaRust`.

#### Option 2: Using Docker 🐳

If you prefer containerization and have Docker installed:

1.  **🧱 Build the Docker Image:**
    From the repository’s root directory:
    ```bash
    docker build -t dynarust:latest .
    ```
    This command builds a container image named `dynarust` with the tag `latest`.
---

## ▶️ Running DynaRust

After building (or using Docker), start DynaRust nodes using command‑line arguments to set the listening address and (optionally) join an existing cluster.

**Command Syntax:**

```bash
./target/release/DynaRust <LISTEN_ADDRESS> [JOIN_ADDRESS]
```

*   **`LISTEN_ADDRESS` (Required):** 📡 The IP address and port for this node to listen on (e.g., `127.0.0.1:6660` for local, `0.0.0.0:6660` for all interfaces).
*   **`JOIN_ADDRESS` (Optional):** 🔗 The IP address and port of an existing DynaRust node to join. Omit this to start a new cluster.

**Example 1: Start the First Node 🟢**

Starts a single node locally on port 6660, forming a new cluster:
```bash
./target/release/DynaRust 127.0.0.1:6660
```
*Check the terminal for output logs 📝.*

**Example 2: Start a Second Node and Join the First 🔗**

Starts a second node on port 6661 and joins the cluster via the first node (`127.0.0.1:6660`):
```bash
./target/release/DynaRust 127.0.0.1:6661 127.0.0.1:6660
```
*The new node synchronizes its state with the cluster 🤝.*

Add more nodes by providing unique `LISTEN_ADDRESS` values and specifying any existing node as the `JOIN_ADDRESS`.

---

## 📡 Using the API

Interact with your DynaRust cluster using standard HTTP requests. Send requests to *any* node; the system handles routing and consistency internally.

**❗️ Important Notes:**

*   Replace `localhost:6660` in examples with the actual `LISTEN_ADDRESS` of a running node.
*   Replace `{table}` with your desired table name (e.g., `default`, `users`, `products`). If omitted, the `"default"` table is used implicitly in older versions, but explicit use is recommended.
*   For `PUT` requests, the body **must** be JSON (e.g., `{"value": "your-data"}`) and the `Content-Type: application/json` header must be set.
---
## 🆘 Troubleshooting

Encountering issues? Check these common points:

1.  **❌ Node Fails to Join Cluster:**
    *   **Target Node Running?** Is the node at `JOIN_ADDRESS` active?
    *   **Network:** Can the joining node reach the `JOIN_ADDRESS` (IP & port)?
        *   *Native:* Use `ping <ip>` and `nc -zv <ip> <port>`.
        *   *Docker:* Are containers on the same `docker network`? Use container names (e.g., `dynarust-node1:6660`). Check `docker logs <container_name>`.
    *   **Address/Port Match?** Double-check `LISTEN_ADDRESS` and `JOIN_ADDRESS`.
    *   **Logs:** Check logs on *both* nodes (see Debugging below).

2.  **❓ Data Not Synchronizing / Inconsistent Reads:**
    *   **Patience:** Eventual consistency takes time (usually milliseconds to seconds). Allow a brief moment for gossip.
    *   **Membership:** Check cluster view: `curl http://<any-node-address>/membership`. Are all nodes listed?
    *   **Logs:** Look for network errors or sync issues.

3.  **❓💾 Data Lost After Restart:**
    *   **Permissions:** Does the process have write access to the directory containing `storage.db`? Enough disk space?
    *   **Docker Persistence:** Did you mount a volume correctly (see [Deployment with Docker](#-deployment-with-docker))? Without a volume, data is lost when the container stops.

4.  **🐞 General Debugging:**
    Enable detailed logs using the `RUST_LOG` environment variable:
    *   **Native Execution:**
        ```bash
        RUST_LOG=debug ./target/release/DynaRust <LISTEN_ADDRESS> [JOIN_ADDRESS]
        ```
    *   **Docker Execution:** Add `-e RUST_LOG=debug` to your `docker run` command.
        ```bash
        docker run -d --name dynarust-node1 --network dynanet -p 6660:6660 \
          -v dynarust-data1:/app \
          -e RUST_LOG=debug \
          dynarust:latest 0.0.0.0:6660
        ```
       Then check logs: `docker logs dynarust-node1`

---

With robust real‑time updates (up to **5000 live connections per node** 🔥) and seamless scalability 🚀, DynaRust is ideal for applications needing instantaneous data propagation across distributed environments. Enjoy building! 🎉

---
