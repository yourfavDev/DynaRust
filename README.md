# 🦀 DynaRust: Distributed Key-Value Store

DynaRust is a distributed key‑value store built in Rust 🦀. It's designed to be reliable 💪 and easy to manage, allowing you to add or remove nodes (servers) dynamically without interrupting service 🔄.

Think of it as a shared dictionary 📚 spread across multiple computers 💻↔️💻. You can store data (key‑value pairs), retrieve it, and delete it using a simple web API 🔌. DynaRust automatically copies your data across available nodes for high availability ✅ and synchronizes changes over time (eventual consistency). It stores data in memory for speed ⚡️ and persists it to disk (`storage.db`) 💾 so your data remains safe even if a node restarts.

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

*   **🔒 Cluster Security:**
    Each node requires a "secret" token (set by the `CLUSTER_SECRET` environment variable) to join a cluster.

*   **🌐 Distributed Storage:**
    Data is automatically partitioned and spread across all nodes in the cluster.

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

## 🚀 Getting Started

Follow these steps to get DynaRust running on your local machine or server.

### 🛠️ Prerequisites

Install the following before building or running DynaRust:

*   **🦀 Rust:** Version 1.86.0 or newer. (Includes `cargo`—the Rust package manager and build tool.)
    *   [Install Rust with rustup](https://rustup.rs/).

*   **⚙️ Standard Build Tools:**
    A C compiler (like `gcc`), `make`, and other common build utilities.
    *   On Debian/Ubuntu:
        ```bash
        sudo apt update && sudo apt install build-essential
        ```
    *   On Fedora/CentOS/RHEL:
        ```bash
        sudo dnf groupinstall "Development Tools"
        # or
        sudo yum groupinstall "Development Tools"
        ```

*   **🔑 OpenSSL Development Libraries:**
    *   On Debian/Ubuntu:
        ```bash
        sudo apt install libssl-dev
        ```
    *   On Fedora/CentOS/RHEL:
        ```bash
        sudo dnf install openssl-devel
        # or
        sudo yum install openssl-devel
        ```

*   **🧩 pkg-config:**
    *   On Debian/Ubuntu:
        ```bash
        sudo apt install pkg-config
        ```
    *   On Fedora/CentOS/RHEL:
        ```bash
        sudo dnf install pkgconf-pkg-config
        # or
        sudo yum install pkgconfig
        ```

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

*(See the [🐳 Deployment with Docker](#-deployment-with-docker) section for running the container.)*

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

### API Endpoints

All key‑value operations are scoped under a table name (`{table}`).

1.  **✍️ Store or Update a Value (HTTP PUT):**
    Creates or updates a key's value within `{table}`.
    ```bash
    # Example: Store 'mydata' for key 'mykey' in the 'default' table
    curl -X PUT http://localhost:6660/default/key/mykey \
      -H "Content-Type: application/json" \
      -d '{"value": "mydata"}'
    ```
    *Response: `200 OK` on success.*

2.  **🔍 Retrieve a Value (HTTP GET):**
    Retrieves the value for `{key}` in `{table}`.
    ```bash
    # Example: Get value for 'mykey' in the 'default' table
    curl http://localhost:6660/default/key/mykey
    ```
    *Response: `{"value": "mydata"}` (if found) or `404 Not Found`.*

3.  **🗑️ Delete a Value (HTTP DELETE):**
    Removes `{key}` (and its value) from `{table}`.
    ```bash
    # Example: Delete 'mykey' from the 'default' table
    curl -X DELETE http://localhost:6660/default/key/mykey
    ```
    *Response: `200 OK` (if deleted) or `404 Not Found`.*

4.  **📚 Fetch Entire Table Store (HTTP GET):**
    Returns all key‑value pairs stored in `{table}`.
    ```bash
    # Example: Get all data from the 'default' table
    curl http://localhost:6660/default/store
    ```
    *Response: A JSON object `{ "key1": "value1", "key2": "value2", ... }`.*

5.  **🔑 Retrieve Table Keys (HTTP GET/POST):**
    *   **GET:** Retrieves all keys for `{table}`.
        ```bash
        # Example: Get all keys from the 'default' table
        curl http://localhost:6660/default/keys
        ```
        *Response: `["key1", "key2", ...]`*
    *   **POST:** Retrieve values for multiple specific keys in `{table}`.
        ```bash
        # Example: Get values for 'key1', 'key2', 'key3' from the 'default' table
        curl -X POST http://localhost:6660/default/keys \
          -H "Content-Type: application/json" \
          -d '["key1", "key2", "key3"]'
        ```
        *Response: `{ "key1": "value1", "key2": "value2", ... }` (Keys not found are omitted).*

6.  **🔔 Subscribe to Real‑Time Updates (HTTP GET):**
    Connect using Server‑Sent Events (SSE) for instant updates on `{key}` within `{table}`.
    ```bash
    # Example: Subscribe to updates for 'mykey' in the 'default' table
    curl -N http://localhost:6660/default/subscribe/mykey
    ```
    *Live changes (< 5 ms)! A single node handles up to **5000 live connections** 🔥. Scale by adding nodes!*

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
+--------+       +-------------------+       +-----------------+
| Client | ----> | Node API Endpoint | ----> | In-Memory Store | ----+
|   👤   |       | (Processes Req.)  |       |  (Local State)💭|     |
+--------+       +-------------------+       +-----------------+     |
     ^             (HTTP: PUT/GET/DEL)           |                 v
     |                                             |         (Periodic Save)
     | (Gossip: Membership & State 🔄)       +--------------+   +-----------------+
     +-------- Cluster Synchronization ----- | Other Nodes  |   | Disk Persistence|
             (Eventual Consistency)         |    💻↔️💻     |   |  (storage.db) 💾|
                                            +--------------+   +-----------------+
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
