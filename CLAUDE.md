# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Architectural Overview

The codebase appears to be a Rust project, heavily focused on network communication, security, storage, and tokenization, suggesting an application that handles data processing, potentially involving encryption or large data structures (indicated by `zstd` dependencies).

**Major Components & Interactions:**

1.  **Network Layer (`src/network/`)**:
    *   `broadcaster.rs` and `mod.rs` suggest a component responsible for broadcasting information across the system. This likely interfaces with other layers to disseminate data or state updates.
2.  **Security Layer (`src/security/`)**:
    *   `authentication.rs` and `mod.rs` handle user or service authentication/authorization. This layer is critical for securing interactions with storage and network components. It interacts heavily with encryption mechanisms.
3.  **Storage Layer (`src/storage/`)**:
    *   This is the data persistence core, featuring several modules:
        *   `persistance.rs`: Likely handles the primary read/write operations for application data.
        *   `engine.rs`: Suggests a core logic engine for data manipulation or state management within storage.
        *   `subscription.rs`: Implies features related to managing subscriptions or access control over stored data.
        *   `snapshoting.rs`: Points to functionality for creating snapshots of the storage state, crucial for backup or versioning.
        *   `statistics.rs`: Suggests metrics collection related to storage operations.
        *   `admin.*` (`admin.html`, `admin.rs`): Indicates an administrative interface or endpoint is present.
4.  **Tokenizer Layer (`src/tokenizer/`)**:
    *   `engine.rs` and `mod.rs` form a dedicated component for text processing, likely involved in data preparation, compression, or encoding before storage or transmission.
5.  **Core / Utilities**:
    *   The presence of `encryption.sh` and key files (`encryption.key`) indicates that cryptography is an integral part of the system's operation, likely used by the Security and Storage layers.
    *   Files in the root like `Cargo.toml` and `.env` define project dependencies and environment configuration.

**Component Interaction Flow (Hypothetical):**

A typical flow might involve: A network request hits the **Network Layer**, which is validated by the **Security Layer**. If authorized, data is processed by the **Tokenizer Layer** (for encoding/compression) before being persisted via the **Storage Layer's engine**. The Storage Layer may use **Snapshotting** for state management and the Statistics module to track performance.

## Common Commands

### Build
*   **Command**: `cargo build`
*   **Notes**: Standard Rust compilation. Check `Cargo.toml` for specific target configurations or dependencies required by `zstd-sys`.

### Linting
*   **Command**: (Not explicitly found, but typically handled by `cargo clippy`)
*   **Notes**: Check for existing linting setup in a `.cargo/config.toml` file or look for integration with Clippy via scripts in `test.sh`.

### Testing
*   **Command**: `./test.sh` (or individual tests)
*   **Notes**: The presence of `test.sh` suggests an explicit script for running the test suite. Review files in `tests/` for component-specific test logic, especially around storage engine and network handlers.

### Running Application / Stress Testing
*   **Command**: `./stress.sh`
*   **Notes**: This script is designated for stress testing the system, likely targeting the Storage or Network layers under heavy load. Review this script to understand the specific load profile it imposes on the system components.

## Code Conventions & Specifics

*   **Encryption**: Cryptographic operations are separated (via `encryption.sh` and key files), suggesting a clear separation of cryptographic concerns from business logic in storage/security modules.
*   **Data Structures**: The presence of `zstd` related files (`zdict.h`, `zstd.h`) suggests that data serialization or compression is a significant concern, likely implemented within the Tokenizer layer or Storage persistence routines.
*   **Administration**: The existence of `src/storage/admin.rs` and `src/storage/admin.html` indicates that system configuration or monitoring access points are designed into the storage module structure.