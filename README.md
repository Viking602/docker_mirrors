# Docker Registry Mirror

A Docker registry mirror built with Rust and Actix-web that can proxy requests to various container registries.

## Supported Registries

- docker.io
- quay.io
- gcr.io
- k8s.gcr.io
- registry.k8s.io
- ghcr.io
- docker.cloudsmith.io
- nvcr.io
- gitlab.com

## Features

- Proxies requests to multiple container registries
- Supports all HTTP methods (GET, POST, PUT, DELETE, HEAD, PATCH)
- Forwards headers and request bodies
- Modular architecture for easy maintenance and extension

## Project Structure

The project is organized into several modules:

- `src/config/` - Configuration for supported registries
- `src/handlers/` - HTTP request handlers
- `src/models/` - Data structures
- `src/services/` - Business logic for proxying requests
- `src/utils/` - Utility functions for logging, etc.

## Usage

### Building with Cargo

```bash
cargo build --release
```

### Running with Cargo

```bash
./target/release/docker_mirrors
```

### Using Docker

#### Building the Docker image

```bash
docker build -t docker-registry-mirror .
```

#### Running the Docker container

```bash
docker run -p 8080:8080 docker-registry-mirror
```

The server will start on `0.0.0.0:8080`.

### Making Requests

To pull an image from Docker Hub:

```bash
docker pull localhost:8080/docker/library/ubuntu:latest
```

To pull an image from Quay.io:

```bash
docker pull localhost:8080/quay/prometheus/prometheus:latest
```

## Configuration

The supported registries are configured in `src/config/mod.rs`. You can modify this file to add or remove registries.

## License

MIT
