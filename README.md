Below is the updated README with enhanced emphasis on real‑time updates and performance:

---

# DynaRust: A Simple Distributed Key-Value Store

DynaRust is a distributed key‑value store built in Rust. It's designed to be reliable and easy to manage, allowing you to add or remove nodes (servers) dynamically without interrupting service.

Imagine it as a shared dictionary spread across multiple computers. You can store data (key‑value pairs), retrieve it, and delete it using a simple web API. DynaRust automatically copies your data across available nodes for high availability and synchronizes changes over time (eventual consistency). It stores data in memory for speed and persists it to disk (`storage.db`) so your data remains safe even if a node restarts.

With its advanced real‑time update capabilities, DynaRust pushes live changes with latencies below 5 ms. In fact, on a typical VPS with 1 GB of RAM and a 100 Mbps bandwidth connection, a node can comfortably sustain peak traffic of up to **5000 live connections**—and you can increase capacity even further simply by adding more nodes to your cluster.

---

## Key Features

- **HOT RELOAD & REAL‑TIME UPDATES:**  
  Enjoy lightning‑fast, real‑time updates using Server‑Sent Events (SSE). Subscribe to a key with:
  ```bash
  curl -N http://localhost:8080/{TABLE}/subscribe/{KEY}
  ```  
  Changes are pushed  instantly and on a standard VPS (1GB RAM, 100MBPS), a single node can handle up to **5000 simultaneous live connections**. Need more capacity? Just add more nodes to the cluster.

- **Cluster Security:**  
  Each node requires a "secret" token (set by the `CLUSTER_SECRET` environment variable) to join a cluster.

- **Distributed Storage:**  
  Data is automatically partitioned and spread across all nodes in the cluster.

- **High Availability:**  
  If one node fails, the remaining nodes continue to serve requests for the available data.

- **Dynamic Cluster Membership:**  
  Nodes can join or leave the cluster seamlessly without manual re‑configuration.

- **Automatic State Sync:**  
  New or returning nodes fetch the latest state from the cluster automatically.

- **Persistent Storage:**  
  Data is saved to a local `storage.db` file to prevent data loss upon node restarts.

- **RESTful API:**  
  A simple HTTP interface (using GET, PUT, DELETE) makes it easy to interact with your data.

---

## Getting Started

Follow these steps to get DynaRust running on your local machine or server.

### Prerequisites

Install the following before building or running DynaRust:

- **Rust:** Version 1.86.0 or newer. (Includes `cargo`—the Rust package manager and build tool.)  
  [Install Rust with rustup](https://rustup.rs/).

- **Standard Build Tools:**  
  A C compiler (like `gcc`), `make`, and other common build utilities.
    - On Debian/Ubuntu:
      ```bash
      sudo apt update && sudo apt install build-essential
      ```
    - On Fedora/CentOS/RHEL:
      ```bash
      sudo dnf groupinstall "Development Tools"
      ```  
      or
      ```bash
      sudo yum groupinstall "Development Tools"
      ```

- **OpenSSL Development Libraries:**
    - On Debian/Ubuntu:
      ```bash
      sudo apt install libssl-dev
      ```
    - On Fedora/CentOS/RHEL:
      ```bash
      sudo dnf install openssl-devel
      ```  
      or
      ```bash
      sudo yum install openssl-devel
      ```

- **pkg-config:**
    - On Debian/Ubuntu:
      ```bash
      sudo apt install pkg-config
      ```
    - On Fedora/CentOS/RHEL:
      ```bash
      sudo dnf install pkgconf-pkg-config
      ```  
      or
      ```bash
      sudo yum install pkgconfig
      ```

### Installation

You can either build DynaRust directly from source or use a pre‑built Docker image.

#### Option 1: Build from Source

1. **Clone the Repository:**
   ```bash
   git clone https://github.com/yourfavDev/DynaRust
   cd DynaRust
   ```  
   *(Replace the URL with the actual repository location if needed.)*

2. **Build the Project:**  
   Compile the Rust code for an optimized release build:
   ```bash
   cargo build --release
   ```  
   The final binary is located at `target/release/DynaRust`.

#### Option 2: Using Docker

If you prefer containerization and have Docker installed:

1. **Build the Docker Image:**  
   From the repository’s root directory:
   ```bash
   docker build -t dynarust:latest .
   ```
   This command builds a container image named `dynarust` with the tag `latest`.

*(See the [Deployment with Docker](#deployment-with-docker) section for running the container.)*

---

## Running DynaRust

After building (or using Docker), start DynaRust nodes using command‑line arguments to set the listening address and (optionally) join an existing cluster.

**Command Syntax:**

```bash
./target/release/DynaRust <LISTEN_ADDRESS> [JOIN_ADDRESS]
```

- **`LISTEN_ADDRESS` (Required):**  
  The IP address and port for this node to listen for incoming API requests and cluster communication. For example, `127.0.0.1:6660` for local testing or `0.0.0.0:6660` to accept connections from any network interface.

- **`JOIN_ADDRESS` (Optional):**  
  The IP address and port of an existing DynaRust node. If provided, this node will contact the `JOIN_ADDRESS` to join the cluster and synchronize state. If omitted, it starts a new cluster.

**Example 1: Start the First Node**

This starts a single node on the local machine at port 6660, forming a new cluster:
```bash
./target/release/DynaRust 127.0.0.1:6660
```
*The terminal will display output logs.*

**Example 2: Start a Second Node and Join the First**

This starts a second node on port 6661 and has it join the cluster via the first node (`127.0.0.1:6660`):
```bash
./target/release/DynaRust 127.0.0.1:6661 127.0.0.1:6660
```
*After initialization, the new node synchronizes its state with the cluster.*

You can add additional nodes by providing unique `LISTEN_ADDRESS` values and specifying any existing node as `JOIN_ADDRESS`.

---

## Using the API

Interact with your DynaRust cluster using standard HTTP requests. You can send requests to *any* node; the system handles routing and data consistency internally.

**Note:**
- Replace `localhost:6660` in examples with the actual `LISTEN_ADDRESS` of a running node.
- For PUT requests, the request body **must** be JSON (e.g., `{"value": "your-data"}`) and the `Content-Type` header set appropriately.

Below is an updated version of the API endpoints documentation that clearly shows the use of the `{table}` segment in all key‑value related endpoints. In these examples, replace `{table}` with your desired table name (for example, `"default"`) when making requests.

---

## Using the API

All key‑value operations are now scoped under a table. If you don't specify a table, the system uses the default table (named `"default"`). Replace `{table}` in the URLs with the name of your table. For example, to work in the default table, use `default` in place of `{table}`.

1. **Store or Update a Value (HTTP PUT):**  
   Creates or updates a key's value within a particular table.
   ```bash
   curl -X PUT http://localhost:6660/default/key/mykey \
     -H "Content-Type: application/json" \
     -d '{"value": "mydata"}'
   ```
   *Expected Response: HTTP 200 OK on success.*

2. **Retrieve a Value (HTTP GET):**  
   Retrieves the value associated with a key in a specific table.
   ```bash
   curl http://localhost:6660/default/key/mykey
   ```
   *Expected Response: `{"value": "mydata"}` if the key exists, or HTTP 404 Not Found if it does not.*

3. **Delete a Value (HTTP DELETE):**  
   Removes a key (and its value) from a table.
   ```bash
   curl -X DELETE http://localhost:6660/default/key/mykey
   ```
   *Expected Response: HTTP 200 OK if deleted, or HTTP 404 Not Found if the key wasn't present.*

4. **Fetch the Entire Store for a Table (HTTP GET):**  
   Returns all key‑value pairs stored in the specified table.
   ```bash
   curl http://localhost:6660/default/store
   ```
   *Expected Response: A JSON object containing all key‑value entries for the table.*

5. **Retrieve Table Keys (HTTP GET/POST):**
    - **GET Request:** Retrieves all keys for a table.
      ```bash
      curl http://localhost:6660/default/keys
      ```
    - **POST Request:** Retrieve multiple keys by providing a list.
      ```bash
      curl -X POST http://localhost:6660/default/keys \
        -H "Content-Type: application/json" \
        -d '["key1", "key2", "key3"]'
      ```

6. **Subscribe to Real‑Time Updates (HTTP GET):**  
   Connect using Server‑Sent Events (SSE) to receive instant updates for a key within a table.
   ```bash
   curl -N http://localhost:6660/default/subscribe/mykey
   ```
   *Changes on the key are pushed in less than 5 ms, and a single node on a VPS (1GB RAM, 100Mbps) can handle up to 5000 live connections. Scaling capacity is as easy as adding more nodes to the cluster.*

---
## How it Works (Conceptual Overview)

DynaRust is a distributed system that achieves eventual consistency through a state synchronization process:

1. **Client Request:**  
   A client sends an HTTP request (e.g., PUT `/key/mykey`) to any node in the cluster.

2. **Local Processing:**  
   The receiving node:
    - For PUT/DELETE: Updates its in‑memory key‑value store immediately.
    - For GET: Reads the value from memory.

3. **Persistence:**  
   The node periodically writes its in‑memory state to a local `storage.db` file, ensuring data durability across restarts.

4. **Cluster Synchronization (Gossip Protocol):**
    - **Membership:** Nodes periodically exchange messages about cluster membership.
    - **State:** Updates are eventually gossiped throughout the cluster.  
      *Real‑time updates: Subscribed clients receive live changes via SSE in less than \(5\,\text{ms}\).*

      **Performance Highlight:**  
      On a typical VPS with 1GB of RAM and a 100Mbps bandwidth connection, a node can handle up to **5000 concurrent live SSE connections**. Adding more nodes to your cluster scales the capacity seamlessly.

5. **Joining:**  
   A new node contacts an existing node (using the `JOIN_ADDRESS`), retrieves the current membership and state, then joins the cluster to serve requests.

```ascii
+--------+       +-------------------+       +-----------------+
| Client | ----> | Node API Endpoint | ----> | In-Memory Store | ----+
+--------+       +-------------------+       +-----------------+     |
     ^             (Processes Request)         (Local State)         |
     |                                             |                 v
     |                                             v         (Periodic Save)
     | (Gossip: Membership & State)         +--------------+   +-----------------+
     +-------- Cluster Synchronization ----- | Other Nodes  |   | Disk Persistence|
             (Eventual Consistency)         +--------------+   +---(storage.db)--+
```

---

## Deployment with Docker

Docker simplifies deployment by providing a consistent environment and handling dependency management.

### Prerequisites

- **Docker:** Version 20.10 or newer installed and running.
- **Network Connectivity:** Ensure that multiple DynaRust containers on the same Docker network can communicate on the configured `LISTEN_ADDRESS` port (default 6660 inside the container).

### Steps

1. **Build the Image:**  
   If you haven’t already built it:
   ```bash
   docker build -t dynarust:latest .
   ```

2. **Run the Container(s):**

    - **Running the First Node:**  
      Start the first node in a new cluster:
      ```bash
      # Start the first node in detached mode, name it 'dynarust-node1',
      # map host port 6660 to container port 6660.
      docker run -d --name dynarust-node1 -p 6660:6660 \
        dynarust:latest 0.0.0.0:6660
      ```
      The node listens on all interfaces (0.0.0.0) on port 6660.

    - **Running Additional Nodes:**  
      To add more nodes, specify the `JOIN_ADDRESS`. For example:
      ```bash
      # Start a second node, name it 'dynarust-node2', mapping host port 6661
      # to container port 6660. It joins the existing cluster via 'dynarust-node1:6660'.
      docker run -d --name dynarust-node2 -p 6661:6660 \
        dynarust:latest 0.0.0.0:6660 dynarust-node1:6660
      ```

   **Important Docker Networking Notes:**
    - For inter-container communication, create a custom network:
      ```bash
      docker network create dynanet
      docker run -d --name dynarust-node1 --network dynanet -p 6660:6660 \
        dynarust:latest 0.0.0.0:6660
      docker run -d --name dynarust-node2 --network dynanet -p 6661:6660 \
        dynarust:latest 0.0.0.0:6660 dynarust-node1:6660
      ```
    - **Data Persistence:**  
      By default, the `storage.db` file is stored inside the container, which is ephemeral. To persist data, mount a Docker volume:
      ```bash
      docker run -d --name dynarust-node1 --network dynanet -p 6660:6660 \
        -v dynarust-data1:/app \
        dynarust:latest 0.0.0.0:6660
      ```
      *(Adjust the mount path `/app` if needed.)*

---

## Troubleshooting

If you run into issues, consider the following tips:

1. **Node Fails to Join Cluster:**
    - **Verify Target Node:** Ensure the node at `JOIN_ADDRESS` is running and accessible.
    - **Network Connectivity:**
        - *Native:* Use tools like `ping` or `nc -zv <ip-address> <port>`.
        - *Docker:* Verify containers are on the same network and use container names for DNS.
    - **Address Mismatch:** Double‑check the `LISTEN_ADDRESS` and `JOIN_ADDRESS`.
    - **Check Logs:** Look at the logs of both the joining and target nodes (enable debug logging with `RUST_LOG=debug`).

2. **Data Not Synchronizing / Inconsistent Reads:**
    - **Eventual Consistency:** Allow a few seconds for updates to propagate.
    - **Check Membership:** Run `curl http://<node-address>/membership` to ensure all nodes are recognized.
    - **Review Logs:** Investigate any error messages related to synchronization or network issues.

3. **Data Lost After Restart:**
    - **Disk Persistence:**
        - Verify that the user has write permissions and sufficient disk space.
        - In Docker, ensure you mount a volume to persist `storage.db`.

4. **General Debugging:**  
   Enable debug logs for detailed information:
    - **Native Execution:**
      ```bash
      RUST_LOG=debug ./target/release/DynaRust <LISTEN_ADDRESS> [JOIN_ADDRESS]
      ```
    - **Docker Execution:**
      ```bash
      docker run -d --name dynarust-node1 -p 6660:6660 \
        -e RUST_LOG=debug \
        dynarust:latest 0.0.0.0:6660
      ```
   Check container logs (`docker logs dynarust-node1`) or terminal output.

---

With its robust real‑time update capabilities—handling up to **5000 live connections per node** on a modest VPS—and seamless scalability, DynaRust is ideal for applications needing instantaneous data propagation across distributed environments. Enjoy building high‑performance, real‑time systems with DynaRust!

---

Feel free to ask if you have any questions or need further customizations.