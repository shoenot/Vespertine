# Nuke built-in rules and variables.
MAKEFLAGS += -rR --silent
.SUFFIXES:

# Every userland package owns a Makefile. Adding a new application only
# requires adding its directory here (Cargo workspace membership) and giving
# it a Makefile that includes ../app.mk.
PACKAGES := $(sort $(patsubst %/Makefile,%,$(wildcard */Makefile)))

.PHONY: all package clean
all: package

package:
	for package in $(PACKAGES); do \
		echo "[INFO] Packaging userland application: $$package"; \
		$(MAKE) -C $$package package || exit 1; \
	done

clean:
	for package in $(PACKAGES); do \
		$(MAKE) -C $$package clean || exit 1; \
	done
