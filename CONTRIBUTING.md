# Contributing to DAO

Thank you for your interest in contributing to DAO! This document provides instructions for setting up your development environment and the general workflow.

## Prerequisites

- **Rust**: You need the latest stable version of Rust. Install it via [rustup](https://rustup.rs/):
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  ```
- **Node.js** (Optional): Only required if you are working on the npm wrapper or installer scripts.

## Development Workflow

### 1. Clone the Repository

```bash
git clone https://github.com/ShaileshRawat1403/dao.git
cd dao
```

### 2. Build the Project

DAO is a Cargo workspace. You can build all crates from the root:

```bash
cargo build
```

To build for release:

```bash
cargo build --release
```

### 3. Run Tests

We maintain a high standard of test coverage for core logic.

```bash
# Run all tests
cargo test

# Run tests for a specific package
cargo test -p dao-core
cargo test -p dao-exec
```

### 4. Run the CLI Locally

You can run the CLI directly via `cargo run`:

```bash
# Print help
cargo run -p dao-cli -- --help

# Run a workflow on a local repo
cargo run -p dao-cli -- run --repo /path/to/target/repo
```

### 5. Code Style

Please ensure your code is formatted and linted before submitting a PR.

```bash
cargo fmt
cargo clippy -- -D warnings
```

## Project Structure

- **`crates/dao-core`**: The brain. Pure, deterministic state machine and event sourcing logic.
- **`crates/dao-exec`**: The hands. Handles side effects, tool execution, and file system interactions.
- **`crates/dao-cli`**: The face. Command-line interface entry point.

## License

By contributing, you agree that your contributions will be licensed under the MIT License.
