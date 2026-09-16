set -euo pipefail

cargo test --locked -p spirit-core -p spirit-index -p spirit-routing -p spirit-schema -p spirit-sdk --lib
cargo check --locked --workspace --all-targets
