```markdown
# DynaRust: A Simple Distributed Key-Value Store

DynaRust is a distributed key-value store built in Rust. It's designed to be reliable and easy to manage, allowing you to add or remove nodes (servers) dynamically without interrupting service.

Think of it as a shared dictionary spread across multiple computers. You can store data (key-value pairs), retrieve it, and delete it using a simple web API. DynaRust automatically copies your data across the available nodes for high availability and synchronizes changes, ensuring data consistency over time (eventual consistency). It stores data in memory for speed and saves it to disk (`storage.db`) so your data isn't lost if a node restarts.

**Key Features:**

*   **Distributed Storage:** Data is spread across all nodes.
*   **High Availability:** If one node fails, others can still serve requests.
*   **Dynamic Cluster Membership:** Nodes can join or leave the cluster easily.
*   **Automatic State Sync:** New or returning nodes automatically get the latest data.
*   **Persistent Storage:** Data is saved to disk for durability.
*   **RESTful API:** Simple HTTP interface for interacting with your data.

## Getting Started

Follow these steps to get DynaRust running.

### Prerequisites

You'll need the following installed:

*   Rust (version 1.86.0 or newer)
*   Standard build tools (like `gcc`, `make` - often called `build-essential` on Debian/Ubuntu)
*   OpenSSL development libraries (e.g., `libssl-dev` on Debian/Ubuntu, `openssl-devel` on Fedora/CentOS)
*   `pkg-config`

### Installation

**Option 1: Build from Source**

1.  Clone the repository:
    ```bash
    git clone https://github.com/yourfavDev/DynaRust
    cd dynarust
    ```
2.  Build the project (this creates an executable in `target/release/`):
    ```bash
    cargo build --release
    ```

**Option 2: Using Docker**

If you prefer Docker:

1.  Build the Docker image:
    ```bash
    docker build -t dynarust .
    ```
    *(See the [Deployment with Docker](#deployment-with-docker) section for running instructions).*

### Running DynaRust

The DynaRust executable takes one or two arguments:

1.  `LISTEN_ADDRESS`: The IP address and port this node should listen on (e.g., `127.0.0.1:6660`).
2.  `JOIN_ADDRESS` (Optional): The address of an existing node in the cluster to join (e.g., `127.0.0.1:6660`).

**Example 1: Start the first node**

This node listens on `127.0.0.1` port `6660`.
```bash
./target/release/DynaRust 127.0.0.1:6660
```

**Example 2: Start a second node and join the first one**

This node listens on `127.0.0.1` port `6661` and connects to the first node running at `127.0.0.1:6660` to join the cluster.
```bash
./target/release/DynaRust 127.0.0.1:6661 127.0.0.1:6660
```
*You can add more nodes similarly.*

## Using the API

Interact with DynaRust using simple HTTP requests (e.g., with `curl`). Replace `localhost:6660` with the address of any node in your cluster.

**Note:** Data is sent and received as JSON. For PUT requests, the body should be `{"value": "your-data"}`.

1.  **Store a value:** (HTTP PUT)
    ```bash
    curl -X PUT http://localhost:6660/key/mykey -H "Content-Type: application/json" -d '{"value": "mydata"}'
    # Expected Response: HTTP 200 OK
    ```

2.  **Retrieve a value:** (HTTP GET)
    ```bash
    curl http://localhost:6660/key/mykey
    # Expected Response: {"value": "mydata"}
    ```

3.  **Delete a value:** (HTTP DELETE)
    ```bash
    curl -X DELETE http://localhost:6660/key/mykey
    # Expected Response: HTTP 200 OK
    ```

4.  **View cluster members:** (HTTP GET)
    ```bash
    curl http://localhost:6660/membership
    # Expected Response: A JSON list of node addresses in the cluster, e.g.,
    # ["127.0.0.1:6660", "127.0.0.1:6661"]
    ```

## How it Works (Conceptual Overview)

DynaRust aims for eventual consistency across the cluster. Here's the basic flow:

1.  **Client Request:** A client sends an HTTP request (GET, PUT, DELETE) to any node's API endpoint.
2.  **Local Processing:** The receiving node processes the request, updating its local in-memory store.
3.  **Persistence:** Changes are periodically saved to a local file (`storage.db`) for durability.
4.  **Cluster Synchronization:**
    *   Nodes periodically gossip (share) their list of known members with each other.
    *   When data changes, the update eventually propagates to other nodes during synchronization cycles.
    *   When a new node joins, it contacts an existing node, gets the current cluster membership, and fetches the existing data state to merge with its own.

```ascii
+--------+       +-------------------+       +-----------------+
| Client | ----> | Node API Endpoint | ----> | In-Memory Store | ----+
+--------+       +-------------------+       +-----------------+     |
     ^                                             |                 |
     |                                             v                 v
     |                                      +--------------+   +-----------------+
     +-------- Cluster Synchronization ----- | Other Nodes  |   | Disk Persistence|
                                            +--------------+   +-----------------+
```

## Deployment with Docker

Using Docker simplifies deployment and dependency management.

### Prerequisites

*   Docker (version 20.10 or newer)
*   Network connectivity between Docker containers if running a cluster.

### Steps

1.  **Build the Image:** (If you haven't already)
    ```bash
    docker build -t dynarust:latest .
    ```

2.  **Run the Container(s):**

    *   **First Node:**
        ```bash
        # Runs the first node, mapping container port 6660 to host port 6660
        docker run -d --name dynarust-node1 -p 6660:6660 dynarust:latest 0.0.0.0:6660
        ```
        *   `-d`: Run in detached mode (background).
        *   `--name`: Give the container a recognizable name.
        *   `-p 6660:6660`: Map port 6660 on your host machine to port 6660 inside the container.
        *   `0.0.0.0:6660`: Tells DynaRust inside the container to listen on all network interfaces on port 6660.

    *   **Joining Nodes:**
        You need to tell the new node the address of the first node *as reachable from within the Docker network*. Replace `<first-node-ip-or-hostname>` with the appropriate address. If using Docker's default bridge network, you might use the container name (`dynarust-node1`) or its internal IP.

        ```bash
        # Runs a second node, mapping container port 6660 to host port 6661
        # It joins the cluster via the first node (dynarust-node1:6660)
        docker run -d --name dynarust-node2 -p 6661:6660 \
          dynarust:latest 0.0.0.0:6660 dynarust-node1:6660
        ```
        *   `-p 6661:6660`: Maps host port 6661 to the container's port 6660.
        *   `0.0.0.0:6660`: The address the *new* node listens on inside its container.
        *   `dynarust-node1:6660`: The address of the *first* node (using its container name) that this new node should contact to join the cluster. Docker's internal DNS usually resolves container names.

    **Important:** Ensure your Docker network setup allows containers to reach each other by name or IP address on the specified ports (e.g., 6660 in this case).

## Troubleshooting

1.  **Node Fails to Join Cluster:**
    *   **Verify the target node is running:** `curl http://<join-node-address>/membership` (e.g., `curl http://127.0.0.1:6660/membership`). Does it respond?
    *   **Check Network:** Can the joining node reach the `JOIN_ADDRESS`? Use `ping <join-node-address>` or check firewall rules. If using Docker, ensure containers are on the same network and can communicate.
    *   **Check Logs:** Run the node with debug logging enabled (see below).

2.  **Data Not Synchronizing / Other Issues:**
    *   **Enable Debug Logs:** Set the `RUST_LOG` environment variable before running:
        ```bash
        # For native execution
        RUST_LOG=debug ./target/release/DynaRust <LISTEN_ADDRESS> [JOIN_ADDRESS]

        # For Docker (add -e RUST_LOG=debug to docker run)
        docker run -e RUST_LOG=debug [...] dynarust:latest [...]
        ```
        Check the console output for errors or detailed synchronization messages.
    *   **Check Disk Persistence:** If data isn't saved after restarts, ensure the process has write permissions for the `storage.db` file in its working directory. For Docker, consider using volumes to persist data outside the container.
```
