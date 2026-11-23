install-buildkit:
	apt update
	apt upgrade -y
	apt install -y build-essential make git curl wget sudo
	curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

release:
	cargo build --release

test:
	cargo test

debug:
	cargo build

run-debug:
	./target/debug/feathertail

run-release:
	./target/release/feathertail

run:
	cargo run -- --config ./feathertail.toml

deb:
	./scripts/build-deb.sh

deb-dev:
	PACKAGE_CHANNEL=dev ./scripts/build-deb.sh

# Run the admin UI locally (Windows/macOS/Linux) with fake guests from dev/pve.
# Open http://127.0.0.1:8686/ui/ and log in with "dev-token".
dev:
	FEATHERTAIL_PVE_NODES_DIR=dev/pve cargo run -- --dev --config dev/feathertail.dev.toml
