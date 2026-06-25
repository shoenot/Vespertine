# Nuke built-in rules and variables.
MAKEFLAGS += -rR --silent
.SUFFIXES:

# --- Configuration ---
BIN_NAME    := kernel
KARCH       := x86_64
TARGET_NAME := x86_64-unknown-none
IMAGE_NAME  := $(BIN_NAME)-$(KARCH)
QEMUFLAGS   := -smp 2 -m 2G

# --- Toolchain ---
AS := nasm

KERNEL_ELF := target/$(TARGET_NAME)/release/$(BIN_NAME)


PART_START    := 2048
PART_SECTORS  := 128991

DISK_ASSETS := assets/disk
DISK_STAGE  := target/build_deps/disk
DISK_EXPORT := target/build/disk-extract

.PHONY: all
all: target/build/$(IMAGE_NAME).iso

.PHONY: lib
lib:
	$(MAKE) -C lib

.PHONY: ports
ports: lib
	$(MAKE) -C ports

.PHONY: kernel
kernel:
	$(MAKE) -C kernel

.PHONY: userland
userland:
	$(MAKE) -C userland

.PHONY: run
run: target/build_deps/edk2-ovmf/ovmf-code-x86_64.fd target/build/$(IMAGE_NAME).iso update-disk
	qemu-system-x86_64 \
		-M q35 \
		-drive if=pflash,unit=0,format=raw,file=target/build_deps/edk2-ovmf/ovmf-code-x86_64.fd,readonly=on \
		-cdrom target/build/$(IMAGE_NAME).iso \
		-drive file=target/disk.img,format=raw,id=disk0,if=none \
		-device virtio-blk-pci,drive=disk0,disable-legacy=on \
		-accel kvm \
		-cpu host,migratable=no,+invtsc \
		$(QEMUFLAGS) \
		-serial stdio 

.PHONY: run-debug
run-debug: target/build_deps/edk2-ovmf/ovmf-code-x86_64.fd target/build/$(IMAGE_NAME).iso update-disk
	qemu-system-x86_64 \
		-M q35 \
		-drive if=pflash,unit=0,format=raw,file=target/build_deps/edk2-ovmf/ovmf-code-x86_64.fd,readonly=on \
		-cdrom target/build/$(IMAGE_NAME).iso \
		-drive file=target/disk.img,format=raw,id=disk0,if=none \
		-device virtio-blk-pci,drive=disk0,disable-legacy=on \
		-accel kvm \
		-cpu host,migratable=no,+invtsc \
		$(QEMUFLAGS) -no-reboot -no-shutdown -d int -D qemu_idt.log -s -S \
		-serial stdio 

.PHONY: run-bios
run-bios: target/build/$(IMAGE_NAME).iso
	qemu-system-x86_64 \
		-M q35 \
		-cdrom target/build/$(IMAGE_NAME).iso \
		-boot d \
		$(QEMUFLAGS)

target/disk.img:
	echo "[INFO] Creating new target/disk.img"
	dd if=/dev/zero of=target/disk.img bs=1M count=64 status=none
	sgdisk -n 1:$(PART_START):$$(($(PART_START) + $(PART_SECTORS) - 1)) -t 1:8300 target/disk.img > /dev/null

.PHONY: stage-disk
stage-disk: userland ports
	echo "[INFO] Mirroring assets/disk into the disk staging tree"
	rm -rf $(DISK_STAGE)
	mkdir -p $(DISK_STAGE)
	cp -a $(DISK_ASSETS)/. $(DISK_STAGE)/
	mkdir -p $(DISK_STAGE)/Devices
	mkdir -p $(DISK_STAGE)/System/Services
	mkdir -p $(DISK_STAGE)/System/Logs

.PHONY: update-disk
update-disk: target/disk.img stage-disk
	echo "[INFO] Rebuilding ext2 partition from $(DISK_STAGE)"
	mkdir -p target/build
	dd if=/dev/zero of=target/build/partition.img bs=512 count=$(PART_SECTORS) status=none
	fakeroot sh -c 'chown -R 0:0 "$(DISK_STAGE)" && mke2fs -q -F -t ext2 -d "$(DISK_STAGE)" target/build/partition.img'
	dd if=target/build/partition.img of=target/disk.img bs=512 seek=$(PART_START) count=$(PART_SECTORS) conv=notrunc status=none
	rm -f target/build/partition.img
	echo "[SUCCESS] target/disk.img updated successfully from $(DISK_STAGE)."

.PHONY: sync-from-disk
sync-from-disk:
	echo "[INFO] Extracting files from ext2 partition to $(DISK_EXPORT)"
	mkdir -p target/build
	rm -rf $(DISK_EXPORT)
	mkdir -p $(DISK_EXPORT)
	dd if=target/disk.img of=target/build/partition.img bs=512 skip=$(PART_START) count=$(PART_SECTORS) status=none
	debugfs -R "rdump / $(DISK_EXPORT)" target/build/partition.img 2>/dev/null || true
	rm -f target/build/partition.img
	echo "[SUCCESS] $(DISK_EXPORT) updated from target/disk.img."

# ISO Creation (Hybrid BIOS/UEFI)
target/build/$(IMAGE_NAME).iso: target/build_deps/limine/limine kernel
	mkdir -p target/build
	rm -rf iso_root
	mkdir -p iso_root/boot/limine
	mkdir -p iso_root/EFI/BOOT
	
	# Copy the kernel from the cargo target directory
	cp $(KERNEL_ELF) iso_root/boot/kernel
	cp assets/limine.conf iso_root/boot/limine/
	
	# x86_64 Specific Limine binaries
	cp target/build_deps/limine/limine-bios.sys target/build_deps/limine/limine-bios-cd.bin target/build_deps/limine/limine-uefi-cd.bin iso_root/boot/limine/
	cp target/build_deps/limine/BOOTX64.EFI iso_root/EFI/BOOT/
	cp target/build_deps/limine/BOOTIA32.EFI iso_root/EFI/BOOT/
	
	xorriso -report_about FAILURE -as mkisofs -b boot/limine/limine-bios-cd.bin \
		-no-emul-boot -boot-load-size 4 -boot-info-table \
		--efi-boot boot/limine/limine-uefi-cd.bin \
		-efi-boot-part --efi-boot-image --protective-msdos-label \
		iso_root -o target/build/$(IMAGE_NAME).iso
	
	./target/build_deps/limine/limine bios-install target/build/$(IMAGE_NAME).iso
	rm -rf iso_root

.PHONY: copy-out
copy-out:
	@if [ -z "$(FILE)" ]; then \
		echo "Usage: make copy-out FILE=path/to/file [OUT=path/on/host]"; \
		exit 1; \
	fi
	mkdir -p target/build
	dd if=target/disk.img of=target/build/partition.img bs=512 skip=$(PART_START) count=$(PART_SECTORS) status=none
	debugfs -R "dump $(FILE) $(or $(OUT),$(notdir $(FILE)))" target/build/partition.img 2>/dev/null || echo "[ERROR] Failed to copy $(FILE) out of image"
	rm -f target/build/partition.img

.PHONY: sync-to-assets
sync-to-assets: sync-from-disk
	echo "[INFO] Syncing persistent changes back to assets/disk/"
	rsync -av --exclude='/Programs' --exclude='/System' $(DISK_EXPORT)/ $(DISK_ASSETS)/

.PHONY: update-programs
update-programs: update-disk

# External Dependencies (Limine and OVMF)
target/build_deps/limine/limine:
	rm -rf target/build_deps/limine
	mkdir -p target/build_deps/limine
	curl -sL https://github.com/limine-bootloader/limine/releases/latest/download/limine-binary.tar.gz | tar -xz --strip-components=1 -C target/build_deps/limine
	$(MAKE) -C target/build_deps/limine

target/build_deps/edk2-ovmf/ovmf-code-x86_64.fd:
	mkdir -p target/build_deps
	curl -L https://github.com/osdev0/edk2-ovmf-nightly/releases/latest/download/edk2-ovmf.tar.gz | tar -xzf - -C target/build_deps/

.PHONY: clean
clean:
	$(MAKE) -C lib clean
	$(MAKE) -C ports clean
	$(MAKE) -C kernel clean
	$(MAKE) -C userland clean
	cargo clean
	rm -rf iso_root target/build

.PHONY: distclean
distclean: clean
	rm -rf target/build_deps/limine target/build_deps/edk2-ovmf ports/mlibc
