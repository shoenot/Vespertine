APP_NAME := vreg

TARGET_NAME    ?= x86_64-unknown-none
CARGO_PACKAGE  ?= $(APP_NAME)
BINARY_NAME    ?= $(APP_NAME)

USERLAND_DIR   := $(abspath ..)
ROOT_DIR       := $(abspath ../..)
ASSETS_ROOT    ?= $(ROOT_DIR)/assets/disk
SYSTEM_CORE_DIR := $(ASSETS_ROOT)/System/Core
BUILT_BINARY   := $(ROOT_DIR)/target/$(TARGET_NAME)/release/$(BINARY_NAME)
RUSTFLAGS      := -C relocation-model=static -C link-arg=-T$(ROOT_DIR)/scripts/userland.ld

.PHONY: all build package clean
all: package

build:
	echo "[INFO] Building system core program: $(APP_NAME)"
	RUSTFLAGS="$(RUSTFLAGS)" cargo build \
		--manifest-path $(ROOT_DIR)/Cargo.toml \
		-p $(CARGO_PACKAGE) \
		--release \
		--target $(TARGET_NAME)

package: build
	echo "[INFO] Packaging $(APP_NAME) into /System/Core"
	mkdir -p $(SYSTEM_CORE_DIR)
	cp $(BUILT_BINARY) $(SYSTEM_CORE_DIR)/$(BINARY_NAME)
	chmod 0755 $(SYSTEM_CORE_DIR)/$(BINARY_NAME)

clean:
	cargo clean --manifest-path $(ROOT_DIR)/Cargo.toml -p $(CARGO_PACKAGE)
