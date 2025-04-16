# DynaRust: A Simple Distributed Key-Value Store

DynaRust is a distributed key-value store built in Rust. It's designed to be reliable and easy to manage, allowing you to add or remove nodes (servers) dynamically without interrupting service.

Think of it as a shared dictionary spread across multiple computers. You can store data (key-value pairs), retrieve it, and delete it using a simple web API. DynaRust automatically copies your data across the available nodes for high availability and synchronizes changes, ensuring data consistency over time (eventual consistency). It stores data in memory for speed and saves it to disk (`storage.db`) so your data isn't lost if a node restarts.

**Key Features:**
*   **Cluster security:** Each node needs to know the "secret" token to join a cluster (CLUSTER_SECRET env var)
*   **Distributed Storage:** Data is spread across all nodes in the cluster.
*   **High Availability:** If one node fails, others can still serve requests for the available data.
*   **Dynamic Cluster Membership:** Nodes can join or leave the cluster easily without manual reconfiguration of others.
*   **Automatic State Sync:** New or returning nodes automatically fetch the latest data state from the cluster.
*   **Persistent Storage:** Data is saved to a local `storage.db` file for durability across restarts.
*   **RESTful API:** Simple HTTP interface (using GET, PUT, DELETE) for interacting with your data.

## Getting Started

Follow these steps to get DynaRust running on your local machine or server.

### Prerequisites

You'll need the following installed on your system before you can build or run DynaRust:

*   **Rust:** Version 1.86.0 or newer. Includes `cargo` (the Rust package manager and build tool). You can install Rust using [rustup](https://rustup.rs/).
*   **Standard Build Tools:** A C compiler (like `gcc`), `make`, and other common build utilities.
    *   On Debian/Ubuntu: `sudo apt update && sudo apt install build-essential`
    *   On Fedora/CentOS/RHEL: `sudo dnf groupinstall "Development Tools"` or `sudo yum groupinstall "Development Tools"`
*   **OpenSSL Development Libraries:** Required for cryptographic operations used in dependencies.
    *   On Debian/Ubuntu: `sudo apt install libssl-dev`
    *   On Fedora/CentOS/RHEL: `sudo dnf install openssl-devel` or `sudo yum install openssl-devel`
*   **pkg-config:** A helper tool used to find compiler and linker flags for libraries.
    *   On Debian/Ubuntu: `sudo apt install pkg-config`
    *   On Fedora/CentOS/RHEL: `sudo dnf install pkgconf-pkg-config` or `sudo yum install pkgconfig`

### Installation

You can either build DynaRust directly from the source code or use a pre-built Docker image.

**Option 1: Build from Source**

1.  **Clone the Repository:** Get the source code from GitHub.
    ```bash
    git clone https://github.com/yourfavDev/DynaRust
    cd DynaRust
    ```
    *(Note: Replace `https://github.com/yourfavDev/DynaRust` with the actual repository URL if different)*

2.  **Build the Project:** Compile the Rust code. This command builds an optimized release executable.
    ```bash
    cargo build --release
    ```
    The final binary will be located at `target/release/DynaRust`.

**Option 2: Using Docker**

If you have Docker installed and prefer containerization:

1.  **Build the Docker Image:** From the root directory of the cloned repository:
    ```bash
    docker build -t dynarust:latest .
    ```
    This command uses the `Dockerfile` in the repository to build a container image named `dynarust` with the tag `latest`.

    *(See the [Deployment with Docker](#deployment-with-docker) section below for instructions on how to run the container).*

### Running DynaRust

Once built (or using Docker), you can start DynaRust nodes. The executable accepts command-line arguments to configure its network address and join an existing cluster.

**Command Syntax:**

```bash
./target/release/DynaRust <LISTEN_ADDRESS> [JOIN_ADDRESS]
```

*   **`LISTEN_ADDRESS` (Required):** The IP address and port where this DynaRust node should listen for incoming API requests and cluster communication (e.g., `127.0.0.1:6660` for local testing, or `0.0.0.0:6660` to listen on all network interfaces).
*   **`JOIN_ADDRESS` (Optional):** The IP address and port of an *existing* DynaRust node already running in the cluster. If provided, this node will contact the `JOIN_ADDRESS` node to join the cluster and synchronize state. If omitted, this node starts as the first node of a new cluster.

**Example 1: Start the First Node**

This command starts a single DynaRust node listening on the local machine at port `6660`. It forms a new cluster by itself.
```bash
./target/release/DynaRust 127.0.0.1:6660
```
*Output logs will appear in the terminal.*

**Example 2: Start a Second Node and Join the First One**

This command starts a second DynaRust node listening on the local machine at port `6661`. It attempts to join the cluster by contacting the first node running at `127.0.0.1:6660`.
```bash
./target/release/DynaRust 127.0.0.1:6661 127.0.0.1:6660
```
*After starting, this node will synchronize its state with the first node.*

*You can start additional nodes by providing a unique `LISTEN_ADDRESS` for each and pointing them to any existing node's address using `JOIN_ADDRESS`.*

## Using the API

Interact with your DynaRust cluster using standard HTTP requests. You can send requests to *any* node in the cluster; the system handles routing and data consistency internally. Common tools like `curl` are suitable for this.

**Note:**
*   Replace `localhost:6660` in the examples below with the actual `LISTEN_ADDRESS` of any running node in your cluster.
*   For storing data (PUT requests), the request body **must** be JSON format: `{"value": "your-data"}`. Set the `Content-Type` header accordingly.

1.  **Store or Update a Value:** (HTTP PUT)
    Creates a new key or updates the value of an existing key.
    ```bash
    curl -X PUT http://localhost:6660/key/mykey -H "Content-Type: application/json" -d '{"value": "mydata"}'
    # Expected Response: HTTP 200 OK (on success)
    ```
    *Replace `mykey` with your desired key and `mydata` with the value you want to store.*

2.  **Retrieve a Value:** (HTTP GET)
    Fetches the value associated with a specific key.
    ```bash
    curl http://localhost:6660/key/mykey
    # Expected Response: {"value": "mydata"} (if key exists)
    # Expected Response: HTTP 404 Not Found (if key doesn't exist)
    ```

3.  **Delete a Value:** (HTTP DELETE)
    Removes a key and its associated value from the store.
    ```bash
    curl -X DELETE http://localhost:6660/key/mykey
    # Expected Response: HTTP 200 OK (if key existed and was deleted)
    # Expected Response: HTTP 404 Not Found (if key didn't exist)
    ```

4.  **View Cluster Membership:** (HTTP GET)
    Retrieves a list of the network addresses of all nodes currently known to be part of the cluster.
    ```bash
    curl http://localhost:6660/membership
    # Expected Response: A JSON array of strings, e.g.,
    # ["127.0.0.1:6660", "127.0.0.1:6661"]
    ```

## How it Works (Conceptual Overview)

DynaRust operates as a distributed system achieving **eventual consistency**. This means that while updates might take a short time to propagate, all nodes will eventually converge to the same data state. Here's a simplified breakdown of the process:

1.  **Client Request:** A client application sends an HTTP request (e.g., PUT `/key/mykey`) to the API endpoint of *any* node in the DynaRust cluster.
2.  **Local Processing:** The node receiving the request processes it immediately:
    *   For PUT/DELETE: It updates its own in-memory key-value map.
    *   For GET: It reads the value from its in-memory map.
3.  **Persistence:** Periodically (or on certain triggers), the node writes its current in-memory state to a local file named `storage.db` in its working directory. This ensures data survives node restarts.
4.  **Cluster Synchronization (Gossip Protocol):**
    *   **Membership:** Nodes periodically exchange messages ("gossip") with a few other random nodes, sharing their view of the current cluster membership list. This way, information about joining or leaving nodes spreads throughout the cluster.
    *   **State:** When a node receives an update (PUT/DELETE), this change is eventually included in synchronization messages sent to other nodes. Over time, all nodes receive the update.
    *   **Joining:** When a new node starts with a `JOIN_ADDRESS`, it contacts that node. It receives the current membership list and requests the full data state from one or more existing nodes. It merges this state into its local store before becoming fully active.

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

## Deployment with Docker

Using Docker provides a consistent environment and simplifies managing dependencies and deployment across different machines.

### Prerequisites

*   **Docker:** Version 20.10 or newer installed and running.
*   **Network Connectivity:** If running multiple DynaRust containers as a cluster, they need to be able to communicate with each other over the network on the configured `LISTEN_ADDRESS` port (default 6660 inside the container). Docker networking features (like user-defined bridge networks) are recommended for this.

### Steps

1.  **Build the Image:** (If you haven't already built it using the steps in the Installation section)
    ```bash
    docker build -t dynarust:latest .
    ```

2.  **Run the Container(s):**

    *   **Running the First Node:**
        This command starts the first node in a new cluster.
        ```bash
        # Runs the first node in detached mode, names it 'dynarust-node1',
        # maps host port 6660 to container port 6660.
        # The node inside the container listens on 0.0.0.0:6660.
        docker run -d --name dynarust-node1 -p 6660:6660 dynarust:latest 0.0.0.0:6660
        ```
        *   `-d`: Run the container in the background (detached mode).
        *   `--name dynarust-node1`: Assigns a convenient name to the container.
        *   `-p 6660:6660`: Maps port `6660` on the host machine to port `6660` inside the container. You can access the API via `http://<host-ip>:6660`.
        *   `dynarust:latest`: Specifies the Docker image to use.
        *   `0.0.0.0:6660`: This is the `LISTEN_ADDRESS` passed to the DynaRust process *inside* the container, telling it to listen on all its network interfaces on port 6660.

    *   **Running Additional Nodes to Join the Cluster:**
        To add more nodes, you need to provide the `JOIN_ADDRESS`. This address must be reachable *from within the Docker network*. Using the container name of the first node (`dynarust-node1`) often works if the containers are on the same Docker network.

        ```bash
        # Runs a second node, names it 'dynarust-node2', maps host port 6661
        # to container port 6660.
        # It listens on 0.0.0.0:6660 inside its container and joins
        # the cluster via the first node (dynarust-node1:6660).
        docker run -d --name dynarust-node2 -p 6661:6660 \
          dynarust:latest 0.0.0.0:6660 dynarust-node1:6660
        ```
        *   `-p 6661:6660`: Maps port `6661` on the host to port `6660` inside *this* container. You access this node's API via `http://<host-ip>:6661`.
        *   `0.0.0.0:6660`: The `LISTEN_ADDRESS` for the second node *inside* its container.
        *   `dynarust-node1:6660`: The `JOIN_ADDRESS`. Docker's internal DNS should resolve the service name `dynarust-node1` to the first container's IP address, targeting port 6660.

    **Important Docker Networking Notes:**
    *   For reliable communication between containers using names (`dynarust-node1`), it's best practice to create a custom Docker network and attach the containers to it:
        ```bash
        docker network create dynanet
        docker run -d --name dynarust-node1 --network dynanet -p 6660:6660 dynarust:latest 0.0.0.0:6660
        docker run -d --name dynarust-node2 --network dynanet -p 6661:6660 dynarust:latest 0.0.0.0:6660 dynarust-node1:6660
        ```
    *   **Persistence with Volumes:** By default, the `storage.db` file is stored inside the container and will be lost if the container is removed. To persist data, use Docker volumes:
        ```bash
        # Example for the first node, mounting a named volume 'dynarust-data1'
        # to the container's working directory '/app' (assuming /app is WORKDIR in Dockerfile)
        docker run -d --name dynarust-node1 --network dynanet -p 6660:6660 \
          -v dynarust-data1:/app \
          dynarust:latest 0.0.0.0:6660
        ```
        *(Adjust the volume mount path `/app` if the working directory in your Dockerfile is different).*

## Troubleshooting

If you encounter issues, here are some common problems and solutions:

1.  **Node Fails to Join Cluster:**
    *   **Symptom:** A new node starts but doesn't appear in the `/membership` list of other nodes, or it logs errors about connection refused/timeout.
    *   **Checks:**
        *   **Verify Target Node:** Is the node specified in `JOIN_ADDRESS` actually running and accessible? Try `curl http://<JOIN_ADDRESS>/membership` from the machine trying to join (or from within the joining container if using Docker). It should respond with a membership list.
        *   **Network Connectivity:** Can the joining node reach the `JOIN_ADDRESS` node on the specified port?
            *   *Native:* Use `ping <ip-address>` (checks basic reachability) and `nc -zv <ip-address> <port>` or `telnet <ip-address> <port>` (checks if the port is open). Check firewalls on both machines.
            *   *Docker:* Ensure containers are on the same Docker network. Use `docker exec <joining-container-name> ping <target-container-name>` or `docker exec <joining-container-name> nc -zv <target-container-name> <port>`. Check Docker network configurations.
        *   **Address Mismatch:** Double-check that the `LISTEN_ADDRESS` and `JOIN_ADDRESS` are correct. If using Docker, ensure you are using the correct internal container names/IPs and ports for the `JOIN_ADDRESS`.
        *   **Check Logs:** Examine the console output of *both* the joining node and the target node for specific error messages. Enable debug logging for more details (see below).

2.  **Data Not Synchronizing / Inconsistent Reads:**
    *   **Symptom:** Reading a key from different nodes yields different results, or data written to one node doesn't appear on others after a short wait.
    *   **Checks:**
        *   **Wait (Eventual Consistency):** Remember that consistency is eventual. Allow a few seconds for synchronization to occur, especially in larger clusters or high-latency networks.
        *   **Check Membership:** Use `curl http://<any-node-address>/membership` on multiple nodes. Do all nodes agree on the cluster membership? If not, there might be a network partition or join issue.
        *   **Check Logs:** Look for errors related to synchronization, serialization, or network communication in the logs of all nodes. Enable debug logging.

3.  **Data Lost After Restart:**
    *   **Symptom:** Data stored previously is gone after stopping and restarting a DynaRust node.
    *   **Checks:**
        *   **Disk Persistence:** The `storage.db` file should be created in the working directory where the `DynaRust` executable was run.
            *   *Native:* Does the user running the process have write permissions in that directory? Is there enough disk space?
            *   *Docker:* Are you using Docker volumes? If not, the data is ephemeral and stored only within the container's temporary filesystem. Ensure you mount a volume to the correct working directory inside the container (e.g., `-v volume-name:/app`) to persist the `storage.db` file.

4.  **General Debugging - Enable Debug Logs:**
    *   To get much more detailed information about internal operations (like synchronization, joining, request processing), run DynaRust with the `RUST_LOG` environment variable set to `debug`.
    *   **Native Execution:**
        ```bash
        RUST_LOG=debug ./target/release/DynaRust <LISTEN_ADDRESS> [JOIN_ADDRESS]
        ```
    *   **Docker Execution:** Add the environment variable using the `-e` flag in `docker run`:
        ```bash
        docker run -d --name dynarust-node1 -p 6660:6660 \
          -e RUST_LOG=debug \
          dynarust:latest 0.0.0.0:6660
        ```
    *   Monitor the container logs (`docker logs dynarust-node1`) or the terminal output for detailed messages.
