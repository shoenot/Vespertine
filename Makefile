# Nuke built-in rules and variables.
MAKEFLAGS += -rR
.SUFFIXES:

# --- Configuration ---
BIN_NAME    := shoes
KARCH       := x86_64
TARGET_NAME := x86_64-unknown-none
IMAGE_NAME  := $(BIN_NAME)-$(KARCH)
QEMUFLAGS   := -m 2G

# --- Toolchain ---
AS := /home/shurjo/build/cross/bin/x86_64-elf-as

# The path where Cargo will output your kernel ELF
KERNEL_ELF := target/$(TARGET_NAME)/release/$(BIN_NAME)

.PHONY: all
all: build/$(IMAGE_NAME).iso

.PHONY: run
run: build_deps/edk2-ovmf/ovmf-code-x86_64.fd build/$(IMAGE_NAME).iso
	qemu-system-x86_64 \
		-M q35 \
		-drive if=pflash,unit=0,format=raw,file=build_deps/edk2-ovmf/ovmf-code-x86_64.fd,readonly=on \
		-cdrom build/$(IMAGE_NAME).iso \
		-accel kvm \
		$(QEMUFLAGS) \
		-serial stdio 

.PHONY: run-debug
run-debug: build_deps/edk2-ovmf/ovmf-code-x86_64.fd build/$(IMAGE_NAME).iso
	qemu-system-x86_64 \
		-M q35 \
		-drive if=pflash,unit=0,format=raw,file=build_deps/edk2-ovmf/ovmf-code-x86_64.fd,readonly=on \
		-cdrom build/$(IMAGE_NAME).iso \
		-accel kvm \
		$(QEMUFLAGS) -d int -no-reboot -M smm=off \
		-serial stdio 

.PHONY: run-bios
run-bios: build/$(IMAGE_NAME).iso
	qemu-system-x86_64 \
		-M q35 \
		-cdrom build/$(IMAGE_NAME).iso \
		-boot d \
		$(QEMUFLAGS)

# --- Assembly Build Step ---
build/idt.o: src/arch/x86_64/interrupts/idt.S
	mkdir -p build/
	$(AS) src/arch/x86_64/interrupts/idt.S -o build/idt.o

.PHONY: kernel
kernel: build/idt.o
	cargo build --release --target $(TARGET_NAME)

# ISO Creation (Hybrid BIOS/UEFI)
build/$(IMAGE_NAME).iso: build_deps/limine/limine kernel
	mkdir -p build
	rm -rf iso_root
	mkdir -p iso_root/boot/limine
	mkdir -p iso_root/EFI/BOOT
	
	# Copy the kernel from the cargo target directory
	cp -v $(KERNEL_ELF) iso_root/boot/kernel
	cp -v build_deps/limine.conf iso_root/boot/limine/
	
	# x86_64 Specific Limine binaries
	cp -v build_deps/limine/limine-bios.sys build_deps/limine/limine-bios-cd.bin build_deps/limine/limine-uefi-cd.bin iso_root/boot/limine/
	cp -v build_deps/limine/BOOTX64.EFI iso_root/EFI/BOOT/
	cp -v build_deps/limine/BOOTIA32.EFI iso_root/EFI/BOOT/
	
	xorriso -as mkisofs -b boot/limine/limine-bios-cd.bin \
		-no-emul-boot -boot-load-size 4 -boot-info-table \
		--efi-boot boot/limine/limine-uefi-cd.bin \
		-efi-boot-part --efi-boot-image --protective-msdos-label \
		iso_root -o build/$(IMAGE_NAME).iso
	
	./build_deps/limine/limine bios-install build/$(IMAGE_NAME).iso
	rm -rf iso_root

# External Dependencies (Limine and OVMF)
build_deps/limine/limine:
	rm -rf build_deps/limine
	mkdir -p build_deps/limine
	curl -sL https://github.com/limine-bootloader/limine/releases/latest/download/limine-binary.tar.gz | tar -xz --strip-components=1 -C build_deps/limine
	$(MAKE) -C build_deps/limine

build_deps/edk2-ovmf/ovmf-code-x86_64.fd:
	mkdir -p build_deps
	curl -L https://github.com/osdev0/edk2-ovmf-nightly/releases/latest/download/edk2-ovmf.tar.gz | tar -xzf - -C build_deps/

.PHONY: clean
clean:
	cargo clean
	rm -rf iso_root build

.PHONY: distclean
distclean: clean
	rm -rf build_deps/limine build_deps/edk2-ovmf
