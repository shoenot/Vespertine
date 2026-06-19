# Nuke built-in rules and variables.
MAKEFLAGS += -rR --silent
.SUFFIXES:

MLIBC_URL    := https://github.com/shoenot/mlibc.git
MLIBC_BRANCH := vespertine
PREFIX       := $(abspath ../target/build_deps/disk/System)
PROGRAMS_DIR := $(abspath ../target/build_deps/disk/Programs)
CLANG_CONFIG := $(abspath x86_64-vespertine.cfg)

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
		-Ddefault_library=shared

mlibc/meson.build:
	if [ ! -d mlibc ]; then \
		echo "[INFO] Cloning mlibc fork"; \
		git clone $(MLIBC_URL) -b $(MLIBC_BRANCH) mlibc; \
	fi

.PHONY: ports
ports: cowsay kilo wc ls grep

.PHONY: grep
grep: zlib
	echo "[INFO] Building port: grep"
	mkdir -p $(PROGRAMS_DIR)
	$(MAKE) -C grep \
		CC='clang --config $(CLANG_CONFIG)'
	cp grep/grep $(PROGRAMS_DIR)/grep

.PHONY: ls
ls:
	echo "[INFO] Building port: ls"
	mkdir -p $(PROGRAMS_DIR)
	$(MAKE) -C ls \
		CC='clang --config $(CLANG_CONFIG)'
	cp ls/ls $(PROGRAMS_DIR)/ls

.PHONY: wc
wc:
	echo "[INFO] Building port: wc"
	mkdir -p $(PROGRAMS_DIR)
	clang --config $(abspath x86_64-vespertine.cfg) -o $(PROGRAMS_DIR)/wc wc/wc.c

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

.PHONY: zlib
zlib:
	echo "[INFO] Building port: zlib"
	$(MAKE) -C zlib \
		CC='clang --config $(CLANG_CONFIG)'
	mkdir -p $(PREFIX)/Libraries
	mkdir -p $(PREFIX)/Headers
	cp zlib/libz.a $(PREFIX)/Libraries/libz.a
	cp zlib/zlib.h $(PREFIX)/Headers/zlib.h
	cp zlib/zconf.h $(PREFIX)/Headers/zconf.h

.PHONY: clean
clean:
	rm -rf mlibc/build
