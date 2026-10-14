.PHONY: build up dist clean check fmt test

build:
	cargo fetch

up:
	cargo run --release -p cadcraft -- --sample

dist:
	cargo build --release -p cadcraft
	strip target/release/cadcraft
	@echo "Binary: target/release/cadcraft"
	@ls -lh target/release/cadcraft

check:
	cargo xtask ci

fmt:
	cargo fmt

test:
	cargo test

clean:
	cargo clean
