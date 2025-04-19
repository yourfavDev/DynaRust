#!/bin/bash

set -e

CERT_DIR="./cert"
mkdir -p "$CERT_DIR"

# 1. Generate CA key and certificate
openssl genrsa -out "$CERT_DIR/ca.key" 4096
openssl req -x509 -new -nodes -key "$CERT_DIR/ca.key" -sha256 -days 3650 \
    -out "$CERT_DIR/ca.crt" \
    -subj "/C=US/ST=State/L=City/O=MyOrg/OU=OrgUnit/CN=MyRootCA"

# 2. Generate server key and CSR
openssl genrsa -out "$CERT_DIR/server.key" 4096
openssl req -new -key "$CERT_DIR/server.key" -out "$CERT_DIR/server.csr" \
    -subj "/C=US/ST=State/L=City/O=MyOrg/OU=OrgUnit/CN=localhost"

# 3. Sign server certificate with CA
openssl x509 -req -in "$CERT_DIR/server.csr" -CA "$CERT_DIR/ca.crt" -CAkey "$CERT_DIR/ca.key" \
    -CAcreateserial -out "$CERT_DIR/server.crt" -days 365 -sha256

# 4. Generate client key and CSR
openssl genrsa -out "$CERT_DIR/client.key" 4096
openssl req -new -key "$CERT_DIR/client.key" -out "$CERT_DIR/client.csr" \
    -subj "/C=US/ST=State/L=City/O=MyOrg/OU=OrgUnit/CN=client"

# 5. Sign client certificate with CA
openssl x509 -req -in "$CERT_DIR/client.csr" -CA "$CERT_DIR/ca.crt" -CAkey "$CERT_DIR/ca.key" \
    -CAcreateserial -out "$CERT_DIR/client.crt" -days 365 -sha256


# 6. Combine client cert and key for clients that need a single PEM
cat "$CERT_DIR/client.crt" "$CERT_DIR/client.key" > "$CERT_DIR/client-combined.pem"
cd $CERT_DIR
openssl pkcs12 -export -out client.p12 -inkey client.key -in client.crt -certfile ca.crt

echo "All certificates and keys have been generated in $CERT_DIR"

