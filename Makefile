setup:
	cargo install --locked flipperzero-tools
	storage mkdir /ext/apps/Me

install:
	cargo build --release
	storage send target/thumbv7em-none-eabihf/release/cold-zero.fap /ext/apps/Me/cold-zero.fap
