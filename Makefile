# Nuke built-in rules and variables.
MAKEFLAGS += -rR
.SUFFIXES:

# --- Configuration ---
BIN_NAME   := shoes
KARCH      := x86_64
TARGET_NAME := x86_64-unknown-none
IMAGE_NAME := $(BIN_NAME)-$(KARCH)
QEMUFLAGS  := -m 2G

# The path where Cargo will output your kernel ELF
KERNEL_ELF := target/$(TARGET_NAME)/release/$(BIN_NAME)

.PHONY: all
all: $(IMAGE_NAME).iso

.PHONY: run
run: edk2-ovmf $(IMAGE_NAME).iso
	qemu-system-x86_64 \
		-M q35 \
		-drive if=pflash,unit=0,format=raw,file=edk2-ovmf/ovmf-code-x86_64.fd,readonly=on \
		-cdrom $(IMAGE_NAME).iso \
		$(QEMUFLAGS) \
		-serial stdio 

.PHONY: run-bios
run-bios: $(IMAGE_NAME).iso
	qemu-system-x86_64 \
		-M q35 \
		-cdrom $(IMAGE_NAME).iso \
		-boot d \
		$(QEMUFLAGS)

# Build the Rust kernel
.PHONY: kernel
kernel:
	cargo build --release --target $(TARGET_NAME)

# ISO Creation (Hybrid BIOS/UEFI)
$(IMAGE_NAME).iso: limine/limine kernel
	rm -rf iso_root
	mkdir -p iso_root/boot/limine
	mkdir -p iso_root/EFI/BOOT
	
	# Copy the kernel from the cargo target directory
	cp -v $(KERNEL_ELF) iso_root/boot/kernel
	cp -v limine.conf iso_root/boot/limine/
	
	# x86_64 Specific Limine binaries
	cp -v limine/limine-bios.sys limine/limine-bios-cd.bin limine/limine-uefi-cd.bin iso_root/boot/limine/
	cp -v limine/BOOTX64.EFI iso_root/EFI/BOOT/
	cp -v limine/BOOTIA32.EFI iso_root/EFI/BOOT/
	
	xorriso -as mkisofs -b boot/limine/limine-bios-cd.bin \
		-no-emul-boot -boot-load-size 4 -boot-info-table \
		--efi-boot boot/limine/limine-uefi-cd.bin \
		-efi-boot-part --efi-boot-image --protective-msdos-label \
		iso_root -o $(IMAGE_NAME).iso
	
	./limine/limine bios-install $(IMAGE_NAME).iso
	rm -rf iso_root

# External Dependencies (Limine and OVMF)
limine/limine:
	rm -rf limine
	mkdir -p limine
	curl -sL https://github.com/limine-bootloader/limine/releases/latest/download/limine-binary.tar.gz | tar -xz --strip-components=1 -C limine
	$(MAKE) -C limine

edk2-ovmf:
	curl -L https://github.com/osdev0/edk2-ovmf-nightly/releases/latest/download/edk2-ovmf.tar.gz | gunzip | tar -xf -

.PHONY: clean
clean:
	cargo clean
	rm -rf iso_root $(IMAGE_NAME).iso

.PHONY: distclean
distclean: clean
	rm -rf limine edk2-ovmf
