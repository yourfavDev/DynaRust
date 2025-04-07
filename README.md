# DynaRust

DynaRust is a distributed key–value store implemented in Rust using Actix Web. Inspired by systems such as DynamoDB, this project demonstrates how to build a scalable, highly available store through the use of consistent hashing, dynamic replication, and request forwarding. Data is stored in memory with persistence to disk ("cold storage") and replicated across the cluster to prevent data loss when nodes go down.

## Table of Contents

- [Overview](#overview)
- [Architecture](#architecture)
    - [Data Flow](#data-flow)
    - [Sequence Diagrams](#sequence-diagrams)
- [Project Structure](#project-structure)
- [Code Samples](#code-samples)
    - [Main Application (main.rs)](#main-application-mainrs)
    - [Engine Module (storage/engine.rs)](#engine-module-storageenginerts)
    - [Persistance Module (storage/persistance.rs)](#persistance-module-storagepersistancerts)
- [API Usage](#api-usage)
- [Running the Cluster](#running-the-cluster)
- [Troubleshooting](#troubleshooting)
- [Conclusion](#conclusion)

## Overview

DynaRust is designed to:
- Distribute data across multiple nodes.
- Use consistent hashing to determine which node (or nodes) are responsible for a specific key.
- Dynamically replicate data based on a configurable replication factor.
- Forward requests automatically if the node receiving the request is not responsible for the key.

In this design, when a client sends a `PUT`, `GET`, or `DELETE` request, the node calculates the responsible node(s) and either processes the request locally or forwards it to the appropriate node(s).

## Architecture

### Data Flow

1. **Client Request:**  
   A client sends a request (e.g., `PUT /key/foo`) to one of the nodes (the coordinator).

2. **Hash Calculation & Target Selection:**  
   The coordinator computes the hash for the key and uses the cluster list to determine the primary target using a modulo operation. To achieve redundancy, a replication factor (e.g., 3) is applied and additional targets are picked in a circular fashion.

3. **Request Forwarding / Local Processing:**
    - If the coordinator is among the selected targets, it processes that part of the request locally.
    - It forwards the request to the remaining target nodes using an HTTP client (e.g. via `reqwest`).

4. **Acknowledgement & Response:**  
   Once all target nodes have processed the request successfully, the coordinator returns a success response to the client.

### Sequence Diagrams

#### Dynamic Replication in a 3-Node Cluster

```plantuml
@startuml
actor Client
participant "Node A\n(localhost:8080)" as A
participant "Node B\n(localhost:8081)" as B
participant "Node C\n(localhost:8082)" as C

Client -> A: PUT /key/foo\nBody: "bar"
A -> A: Compute hash("foo")\nDetermine targets (e.g., Primary = Node B,\nReplica = Node C)
A -> B: Forward PUT /key/foo\nBody: "bar"
A -> C: Forward PUT /key/foo\nBody: "bar"
B --> A: Ack (processed locally)
C --> A: Ack (processed locally)
A --> Client: 201 Key stored
@enduml
