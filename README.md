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
- Handles both direct registry requests (e.g., /docker/...) and Docker Registry API V2 requests (e.g., /v2/...)
- Optimized blob handling with automatic authentication and redirect following
- Improved timeout handling for large blob downloads

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

#### Building the Docker image locally

```bash
docker build -t docker-registry-mirror .
```

#### Running the Docker container

```bash
docker run -p 8080:8080 docker-registry-mirror
```

#### Using pre-built image from GitHub Container Registry

You can also pull the pre-built image from GitHub Container Registry:

```bash
# Using the latest tag
docker pull ghcr.io/Viking602/docker_mirrors:latest
docker run -p 8080:8080 ghcr.io/Viking602/docker_mirrors:latest

# Or using the Releases tag
docker pull ghcr.io/Viking602/docker_mirrors:Releases
docker run -p 8080:8080 ghcr.io/Viking602/docker_mirrors:Releases
```

The server will start on `0.0.0.0:8080`.

### Making Requests

First, configure Docker to use the mirror by adding the following to your Docker daemon configuration file (usually `/etc/docker/daemon.json`):

```json
{
  "registry-mirrors": ["http://localhost:8080"]
}
```

Then restart the Docker daemon:

```bash
sudo systemctl restart docker
```

When configured as a registry mirror, Docker will automatically send requests to the mirror using the Docker Registry API V2 format (e.g., /v2/...). The mirror will recognize these requests and forward them to Docker Hub.

Alternatively, you can pull images directly through the mirror:

To pull an image from Docker Hub:

```bash
docker pull localhost:8080/docker/library/ubuntu:latest
```

To pull an image from Quay.io:

```bash
docker pull localhost:8080/quay/prometheus/prometheus:latest
```

For non-official Docker Hub images, use the namespace:

```bash
docker pull localhost:8080/docker/username/repository:tag
```

## Configuration

The supported registries are configured in `src/config/mod.rs`. You can modify this file to add or remove registries.

### Docker Hub Authentication

To access private repositories or avoid rate limits when pulling from Docker Hub, you can configure Docker Hub credentials using environment variables:

```bash
# When running with Cargo
export DOCKER_HUB_USERNAME=your_username
export DOCKER_HUB_PASSWORD=your_password
./target/release/docker_mirrors

# When running with Docker
docker run -p 8080:8080 \
  -e DOCKER_HUB_USERNAME=your_username \
  -e DOCKER_HUB_PASSWORD=your_password \
  docker-registry-mirror
```

If Docker Hub credentials are not configured, the mirror will attempt to make anonymous requests, which may be subject to rate limits.

### Advanced Request Handling

The mirror includes optimized handling for various Docker registry requests:

#### Blob Requests (Image Layers)

- Automatic pre-authentication for Docker Hub blob requests
- Proper handling of redirects with authentication preserved
- Increased timeouts (5 minutes) for large blob downloads
- Range requests to help with large blobs and network issues
- Detailed logging of rate limit headers when 403 errors occur
- Enhanced CDN fallback mechanism with multiple fallback options:
  - Primary Cloudflare CDN
  - Alternative registry.hub.docker.com CDN
  - Alternative registry-cdn.docker.io CDN
- Intelligent retry logic with exponential backoff
- Multiple User-Agent rotation to avoid filtering
- Last-resort direct download with optimized headers
- Improved header management with Docker-client compatible headers
- Automatic retry with authentication for 401 Unauthorized responses

#### Manifest Requests (Image Metadata)

- Automatic authentication for Docker Hub manifest requests
- Fallback to Docker Hub API for manifest requests that return 403 Forbidden
- Improved path handling for non-library repositories
- Comprehensive Accept headers for all manifest formats

If you encounter 403 Forbidden errors or timeouts when pulling images:

1. Configure Docker Hub credentials as described above to avoid rate limits
2. Check the mirror logs for rate limit information
3. The mirror will automatically attempt to use multiple alternative sources:
   - Multiple CDN fallbacks for blob requests with intelligent retry logic
   - Docker Hub's API for manifest requests
4. Consider using a caching proxy in front of the mirror for frequently accessed content
5. If you're still experiencing issues, try pulling the image directly from Docker Hub first, then try through the mirror

## License

MIT
