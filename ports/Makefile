# Nuke built-in rules and variables.
MAKEFLAGS += -rR --silent
.SUFFIXES:

MLIBC_URL    := https://github.com/shoenot/mlibc.git
MLIBC_BRANCH := vespertine
PREFIX       := $(abspath ../target/build_deps/disk/System)
PROGRAMS_DIR := $(abspath ../target/build_deps/disk/Programs)

.PHONY: all
all: mlibc ports

.PHONY: mlibc
mlibc: mlibc/build/build.ninja
	echo "[INFO] Building mlibc"
	ninja -C mlibc/build
	echo "[INFO] Installing mlibc"
	DESTDIR=$(abspath ../target/build_deps/disk) ninja -C mlibc/build install

mlibc/build/build.ninja: mlibc/meson.build
	echo "[INFO] Configuring mlibc"
	mkdir -p mlibc/build
	meson setup mlibc/build mlibc \
		--cross-file $(abspath cross.ini) \
		--prefix=/System \
		--includedir=Headers \
		--libdir=Libraries \
		-Dlibgcc_dependency=false \
		-Ddefault_library=both

mlibc/meson.build:
	if [ ! -d mlibc ]; then \
		echo "[INFO] Cloning mlibc fork"; \
		git clone $(MLIBC_URL) -b $(MLIBC_BRANCH) mlibc; \
	fi

.PHONY: ports
ports: cat cowsay kilo

.PHONY: cat
cat:
	echo "[INFO] Building port: cat"
	mkdir -p $(PROGRAMS_DIR)
	clang --config $(abspath x86_64-vespertine.cfg) -o $(PROGRAMS_DIR)/cat cat/cat.c

.PHONY: cowsay
cowsay:
	echo "[INFO] Building port: cowsay"
	mkdir -p $(PROGRAMS_DIR)
	clang --config $(abspath x86_64-vespertine.cfg) -o $(PROGRAMS_DIR)/cowsay cowsay/cowsay.c

.PHONY: kilo
kilo:
	echo "[INFO] Building port: kilo"
	mkdir -p $(PROGRAMS_DIR)
	clang --config $(abspath x86_64-vespertine.cfg) -o $(PROGRAMS_DIR)/kilo kilo/kilo.c

.PHONY: clean
clean:
	rm -rf mlibc/build
