# DynaRust: A Distributed Key-Value Store with Automatic Cluster Management

DynaRust is a high-performance distributed key-value store built in Rust that provides automatic cluster management and persistent storage capabilities. The system maintains data consistency across nodes while offering seamless scalability through dynamic node membership.

The store features automatic node discovery and membership synchronization, persistent storage with periodic saves, and a RESTful API for data operations. It leverages the Actix Web framework for high-performance networking and includes built-in failure detection to maintain cluster health. The system supports horizontal scaling through a simple node join mechanism and ensures data durability through periodic persistence to disk.

## Repository Structure
```
.
├── src/                          # Source code directory
│   ├── main.rs                   # Application entry point and server setup
│   ├── logger.rs                 # Logging utilities with colored output
│   ├── network/                  # Network communication components
│   │   ├── broadcaster.rs        # Cluster membership synchronization
│   │   └── mod.rs               # Network module definition
│   └── storage/                  # Storage implementation
│       ├── engine.rs            # Core key-value store logic
│       ├── mod.rs               # Storage module definition
│       └── persistance.rs       # Persistent storage handling
├── docs/                         # Documentation assets
│   ├── infra.dot                # Infrastructure diagram source
│   └── infra.svg                # Infrastructure visualization
├── Cargo.toml                   # Rust package manifest
├── Cargo.lock                   # Dependency lock file
└── Dockerfile                   # Container definition
```

## Usage Instructions
### Prerequisites
- Rust 1.70 or later
- OpenSSL development package
- Docker (optional, for containerized deployment)

### Installation

#### From Source
```bash
# Clone the repository
git clone <repository-url>
cd dynarust

# Build the project
cargo build --release

# Run the server
cargo run --release -- <host:port> [join_node_address]
```

#### Using Docker
```bash
# Build the container
docker build -t dynarust .

# Run a node
docker run -p 6660:6660 dynarust <host:port> [join_node_address]
```

### Quick Start
1. Start the first node:
```bash
cargo run --release -- 127.0.0.1:6660
```

2. Join additional nodes to the cluster:
```bash
cargo run --release -- 127.0.0.1:6661 127.0.0.1:6660
```

3. Interact with the key-value store:
```bash
# Store a value
curl -X PUT http://127.0.0.1:6660/key/mykey -d "myvalue"

# Retrieve a value
curl http://127.0.0.1:6660/key/mykey

# Delete a value
curl -X DELETE http://127.0.0.1:6660/key/mykey
```

### More Detailed Examples
#### Cluster Operations
```bash
# Check cluster membership
curl http://127.0.0.1:6660/membership

# Join a new node to the cluster
curl -X POST http://127.0.0.1:6660/join \
  -H "Content-Type: application/json" \
  -d '{"node": "127.0.0.1:6662"}'
```

### Troubleshooting
#### Common Issues
1. Node Join Failures
   - Error: "Failed to join cluster"
   - Solution: Verify the join node address is correct and the node is running
   - Debug: Check network connectivity and firewall rules

2. Storage Persistence Issues
   - Error: "Error loading DB"
   - Solution: Ensure write permissions for the storage.db file
   - Location: Check the working directory for storage.db

#### Debug Mode
Enable debug logging by setting the RUST_LOG environment variable:
```bash
RUST_LOG=debug cargo run -- <host:port>
```

## Data Flow
DynaRust processes requests through a multi-layer architecture, handling both data operations and cluster management concurrently. Data flows from the HTTP interface through the storage engine and is optionally persisted to disk.

```ascii
Client Request -> HTTP Endpoint -> Storage Engine -> [Persistence Layer]
                                         ↑
                            Cluster Membership Sync
                                         ↓
                              [Other Cluster Nodes]
```

Key component interactions:
- HTTP endpoints receive client requests for data operations
- Storage engine manages in-memory key-value pairs
- Persistence layer periodically saves data to disk
- Membership synchronization maintains cluster state
- Node communication uses HTTP for both data and control plane

## Infrastructure
The application runs as a containerized service:

- Docker::Container (server)
  - Exposes port 6660
  - Requires OpenSSL runtime
  - Persists data to volume-mounted storage

## Deployment
### Prerequisites
- Docker 20.10 or later
- Network access for inter-node communication
- Storage volume for persistence (optional)

### Deployment Steps
1. Build the container:
```bash
docker build -t dynarust:latest .
```

2. Deploy the first node:
```bash
docker run -d \
  --name dynarust-node1 \
  -p 6660:6660 \
  -v dynarust-data:/app/data \
  dynarust:latest
```

3. Join additional nodes:
```bash
docker run -d \
  --name dynarust-node2 \
  -p 6661:6660 \
  -v dynarust-data:/app/data \
  dynarust:latest <host:port> <first-node-address>
```