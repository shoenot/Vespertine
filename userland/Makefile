# Nuke built-in rules and variables.
MAKEFLAGS += -rR --silent
.SUFFIXES:

USER_PROGS  := hesper ns terminal dt dusk con
TARGET_NAME := x86_64-unknown-none
DISK_DIR    := ../target/build_deps/disk/Programs

.PHONY: all
all:
	mkdir -p $(DISK_DIR)
	for prog in $(USER_PROGS); do \
		echo "[INFO] Building userland program: $$prog"; \
		RUSTFLAGS="-C relocation-model=static -C link-arg=-T$(abspath ../scripts/userland.ld)" \
			cargo build --manifest-path ../Cargo.toml -p $$prog --release --target $(TARGET_NAME) || exit 1; \
		cp ../target/$(TARGET_NAME)/release/$$prog $(DISK_DIR)/$$prog; \
	done

.PHONY: clean
clean:
	for prog in $(USER_PROGS); do \
		cargo clean -p $$prog; \
	done
