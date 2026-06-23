# Shared Rust application build and packaging rules.

ifndef APP_NAME
$(error APP_NAME must be set before including ../app.mk)
endif

TARGET_NAME    ?= x86_64-unknown-none
PACKAGE_FORMAT ?= bundle
CARGO_PACKAGE  ?= $(APP_NAME)
BINARY_NAME    ?= $(APP_NAME)
BUNDLE_NAME    ?= $(APP_NAME)
MANIFEST       ?= manifest.toml

USERLAND_DIR   := $(abspath ..)
ROOT_DIR       := $(abspath ../..)
ASSETS_ROOT    ?= $(ROOT_DIR)/assets/disk
PROGRAMS_DIR   := $(ASSETS_ROOT)/Programs
BUILT_BINARY   := $(ROOT_DIR)/target/$(TARGET_NAME)/release/$(BINARY_NAME)
RUSTFLAGS      := -C relocation-model=static -C link-arg=-T$(ROOT_DIR)/scripts/userland.ld

.PHONY: all build package clean
all: package

build:
	echo "[INFO] Building userland application: $(APP_NAME)"
	RUSTFLAGS="$(RUSTFLAGS)" cargo build \
		--manifest-path $(ROOT_DIR)/Cargo.toml \
		-p $(CARGO_PACKAGE) \
		--release \
		--target $(TARGET_NAME)

ifeq ($(PACKAGE_FORMAT),bundle)
BUNDLE_DIR := $(PROGRAMS_DIR)/$(BUNDLE_NAME).app

package: build $(MANIFEST)
	echo "[INFO] Packaging $(APP_NAME) as $(BUNDLE_NAME).app"
	mkdir -p $(BUNDLE_DIR)/bin
	cp $(BUILT_BINARY) $(BUNDLE_DIR)/bin/$(BINARY_NAME)
	chmod 0755 $(BUNDLE_DIR)/bin/$(BINARY_NAME)
	cp $(MANIFEST) $(BUNDLE_DIR)/manifest.toml
else ifeq ($(PACKAGE_FORMAT),flat)
package: build
	echo "[INFO] Packaging bootstrap program: $(BINARY_NAME)"
	mkdir -p $(PROGRAMS_DIR)
	cp $(BUILT_BINARY) $(PROGRAMS_DIR)/$(BINARY_NAME)
	chmod 0755 $(PROGRAMS_DIR)/$(BINARY_NAME)
else
$(error Unsupported PACKAGE_FORMAT '$(PACKAGE_FORMAT)')
endif

clean:
	cargo clean --manifest-path $(ROOT_DIR)/Cargo.toml -p $(CARGO_PACKAGE)
