# Nuke built-in rules and variables.
MAKEFLAGS += -rR --silent
.SUFFIXES:

ABI_HEADER := ../target/build_deps/disk/System/Headers/vespertine/abi.h

.PHONY: all
all: $(ABI_HEADER)

$(ABI_HEADER): vespertine-abi/src/lib.rs vespertine-abi/cbindgen.toml
	echo "[INFO] Generating C headers for vespertine-abi"
	mkdir -p ../target/build_deps/disk/System/Headers/vespertine
	cbindgen vespertine-abi --config vespertine-abi/cbindgen.toml --output $(ABI_HEADER)

.PHONY: clean
clean:
	rm -rf ../target/build_deps/disk/System/Headers/vespertine
