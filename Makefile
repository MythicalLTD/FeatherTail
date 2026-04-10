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
