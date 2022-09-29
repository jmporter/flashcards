# For non-musl, use: armv7-unknown-linux-gnueabihf
TARGET ?= armv7-unknown-linux-musleabihf

DEVICE_IP ?= '10.11.99.1'
DEVICE_HOST ?= root@$(DEVICE_IP)

all: flashcards deploy

flashcards:
	cross build --release --target=armv7-unknown-linux-musleabihf

deploy:
	du -sh ./target/$(TARGET)/release/flashcards
	ssh $(DEVICE_HOST) 'killall -q -9 flashcards || true; systemctl stop xochitl || true'
	scp ./target/$(TARGET)/release/flashcards $(DEVICE_HOST):
	ssh $(DEVICE_HOST) 'RUST_BACKTRACE=1 RUST_LOG=debug ./flashcards'

push:
	du -sh ./target/$(TARGET)/release/flashcards
	scp ./target/$(TARGET)/release/flashcards $(DEVICE_HOST):

run:
	ssh $(DEVICE_HOST) 'killall -q -9 flashcards || true; systemctl stop xochitl || true'
	ssh $(DEVICE_HOST) 'RUST_BACKTRACE=1 RUST_LOG=debug ./flashcards'

start-xochitl:
	ssh $(DEVICE_HOST) 'killall -q -9 flashcards || true; systemctl start xochitl'

stop-xochitl:
	ssh $(DEVICE_HOST) 'killall -q -9 flaschards || true; systemctl stop xochitl'
