# DynaRust: A Distributed Key-Value Store with Dynamic Membership

DynaRust is a distributed key-value store built in Rust that provides reliable data storage with dynamic cluster membership management. The system offers seamless node joining, automatic state synchronization, and persistent storage capabilities.

The system maintains data consistency across nodes through periodic synchronization while providing high availability through its distributed architecture. It features an in-memory store with disk persistence, cluster membership management, and a RESTful API interface for data operations. The implementation leverages Actix Web for HTTP services and supports dynamic cluster topology changes with automatic state merging.

## Repository Structure
```
.
├── src/                    # Source code directory
│   ├── main.rs            # Application entry point with HTTP server and cluster initialization
│   ├── logger.rs          # Logging utilities with colored output
│   ├── network/           # Network communication components
│   │   ├── broadcaster.rs # Cluster membership synchronization
│   │   └── mod.rs        # Network module definition
│   └── storage/           # Storage implementation
│       ├── engine.rs      # Core storage engine
│       ├── mod.rs        # Storage module definition
│       └── persistance.rs # Disk persistence implementation
├── docs/                  # Documentation assets
│   ├── infra.dot         # Infrastructure diagram source
│   └── infra.svg         # Infrastructure visualization
├── Dockerfile            # Multi-stage container build definition
├── Cargo.toml           # Rust package manifest
└── Cargo.lock           # Dependency lock file
```

## Usage Instructions
### Prerequisites
- Rust 1.86.0 or later
- OpenSSL development package
- pkg-config
- Build essentials (for compilation)

### Installation

1. Clone the repository:
```bash
git clone <repository-url>
cd dynarust
```

2. Build the project:
```bash
cargo build --release
```

3. Using Docker:
```bash
docker build -t dynarust .
```

### Quick Start

1. Start a single node:
```bash
./target/release/DynaRust 127.0.0.1:6660
```

2. Start additional nodes and join the cluster:
```bash
./target/release/DynaRust 127.0.0.1:6661 127.0.0.1:6660
```

### More Detailed Examples

1. Store a value:
```bash
curl -X PUT http://localhost:6660/key/mykey -d '{"value": "mydata"}'
```

2. Retrieve a value:
```bash
curl http://localhost:6660/key/mykey
```

3. Delete a value:
```bash
curl -X DELETE http://localhost:6660/key/mykey
```

4. View cluster membership:
```bash
curl http://localhost:6660/membership
```

### Troubleshooting

1. Node Join Issues
- Problem: Node fails to join cluster
- Solution: 
  ```bash
  # Verify the join node is running
  curl http://join-node-address/membership
  # Check network connectivity
  ping join-node-address
  ```

2. Data Synchronization Issues
- Enable debug logging by setting RUST_LOG:
  ```bash
  RUST_LOG=debug ./target/release/DynaRust 127.0.0.1:6660
  ```
- Check storage.db file permissions if persistence fails

## Data Flow
DynaRust implements a distributed key-value store with eventual consistency. Data flows from client requests through the HTTP API, is processed by the storage engine, and is eventually synchronized across all cluster nodes.

```ascii
Client -> HTTP API -> Storage Engine -> Disk Persistence
   ^                       |
   |                      v
   +---- Cluster Sync ----+
```

Key component interactions:
1. HTTP API receives client requests for data operations
2. Storage engine processes operations on local state
3. Periodic sync task broadcasts membership updates
4. Background task persists state to disk
5. Joining nodes merge remote state with local state

## Infrastructure
- server (Docker::Container): Main application container running the DynaRust service
  - Exposes port 6660 for API access
  - Includes OpenSSL runtime dependencies
  - Built using multi-stage build for minimal image size

## Deployment
### Prerequisites
- Docker 20.10 or later
- Network access for container registry

### Deployment Steps
1. Build container:
```bash
docker build -t dynarust:latest .
```

2. Run container:
```bash
# First node
docker run -p 6660:6660 dynarust:latest

# Additional nodes
docker run -p 6661:6660 dynarust:latest 0.0.0.0:6660 first-node:6660
```