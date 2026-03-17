# DynaRust Project

## Project Overview
DynaRust is a distributed key-JSON-value datastore built in Rust. It provides in-memory caching, on-disk persistence, background replication for eventual consistency, and real-time update capabilities using Server-Sent Events (SSE). It handles cluster membership dynamically, allowing nodes to join and synchronize their state.

## Building and Running
*   **Build the project:**
    ```bash
    cargo build --release
    ```
*   **Run a node:**
    ```bash
    # Run the first node
    ./target/release/DynaRust <LISTEN_ADDRESS>

    # Run additional nodes and join the cluster
    ./target/release/DynaRust <LISTEN_ADDRESS> <JOIN_ADDRESS>
    ```

## Development Conventions
*   **Rust & Actix-Web:** Built predominantly with Rust and Actix-Web for high-performance concurrent request handling.
*   **Authentication:** Endpoints (except GET) require JWT token via `Authorization: Bearer <token>`. Node-to-node replication uses a cluster secret defined in `.env`.
*   **Concurrency:** Utilizes `tokio` for async tasks, e.g., snapshots, gossip membership sync, and cluster replication.
*   **Storage Architecture:** Data is stored in-memory using `HashMap<String, HashMap<String, VersionedValue>>` with background disk persistence.

## Key Files
*   `src/main.rs`: Entry point and Actix-Web server initialization.
*   `src/storage/engine.rs`: Core data structures (`VersionedValue`, `AppState`) and CRUD handler logic (`put_value`, `get_value`, `delete_value`).
*   `src/network/broadcaster.rs`: Cluster membership gossip protocol implementation.
*   `src/storage/subscription.rs`: Real-time updates handler using Server-Sent Events (SSE).
