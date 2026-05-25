# Nuke built-in rules and variables.
MAKEFLAGS += -rR --silent
.SUFFIXES:

# --- Configuration ---
BIN_NAME    := kernel
KARCH       := x86_64
TARGET_NAME := x86_64-unknown-none
IMAGE_NAME  := $(BIN_NAME)-$(KARCH)
QEMUFLAGS   := -smp 4 -m 2G

# --- Toolchain ---
AS := nasm

KERNEL_ELF := target/$(TARGET_NAME)/release/$(BIN_NAME)

USER_PROGS := shell hesper

.PHONY: all
all: target/build/$(IMAGE_NAME).iso

.PHONY: run
run: build_deps/edk2-ovmf/ovmf-code-x86_64.fd target/build/$(IMAGE_NAME).iso
	qemu-system-x86_64 \
		-M q35 \
		-drive if=pflash,unit=0,format=raw,file=build_deps/edk2-ovmf/ovmf-code-x86_64.fd,readonly=on \
		-cdrom target/build/$(IMAGE_NAME).iso \
		-accel kvm \
		-cpu host,migratable=no,+invtsc \
		$(QEMUFLAGS) \
		-serial stdio 

.PHONY: run-debug
run-debug: build_deps/edk2-ovmf/ovmf-code-x86_64.fd target/build/$(IMAGE_NAME).iso
	qemu-system-x86_64 \
		-M q35 \
		-drive if=pflash,unit=0,format=raw,file=build_deps/edk2-ovmf/ovmf-code-x86_64.fd,readonly=on \
		-cdrom target/build/$(IMAGE_NAME).iso \
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

##############################
# --- ASSEMBLY FILES HERE ---#
##############################

target/build/gdt.o: kernel/src/arch/x86_64/cpu/gdt.asm
	mkdir -p target/build/
	$(AS) -f elf64 kernel/src/arch/x86_64/cpu/gdt.asm -o target/build/gdt.o
	
target/build/idt.o: kernel/src/arch/x86_64/interrupts/idt.asm target/build/gdt.o
	mkdir -p target/build/
	$(AS) -f elf64 kernel/src/arch/x86_64/interrupts/idt.asm -o target/build/idt.o
	
target/build/switch.o: kernel/src/arch/x86_64/task/switch.asm target/build/idt.o
	mkdir -p target/build/
	$(AS) -f elf64 kernel/src/arch/x86_64/task/switch.asm -o target/build/switch.o

target/build/fpu.o: kernel/src/arch/x86_64/cpu/fpu.asm target/build/switch.o
	mkdir -p target/build/
	$(AS) -f elf64 kernel/src/arch/x86_64/cpu/fpu.asm -o target/build/fpu.o

target/build/syscall.o: kernel/src/arch/x86_64/task/syscall.asm target/build/fpu.o
	mkdir -p target/build/
	$(AS) -f elf64 kernel/src/arch/x86_64/task/syscall.asm -o target/build/syscall.o

.PHONY: kernel
kernel: target/build/syscall.o
	cargo build -p kernel --release --target $(TARGET_NAME)

# Build all userspace programs listed in USER_PROGS with custom userland RUSTFLAGS
.PHONY: userland
userland: scripts/userland.ld
	mkdir -p ramdisk/Programs/
	for prog in $(USER_PROGS); do \
		echo "Building userland program: $$prog"; \
		RUSTFLAGS="-C relocation-model=static -C link-arg=-Tscripts/userland.ld" \
			cargo build -p $$prog --release --target $(TARGET_NAME) || exit 1; \
		cp target/$(TARGET_NAME)/release/$$prog ramdisk/Programs/$$prog; \
	done

##############################
# --- ASSEMBLY FILES DONE ---#
##############################

# ISO Creation (Hybrid BIOS/UEFI)
target/build/$(IMAGE_NAME).iso: build_deps/limine/limine kernel userland
	mkdir -p target/build
	rm -rf iso_root
	tar -cf build_deps/ramdisk.tar -C ramdisk . --format=ustar
	mkdir -p iso_root/boot/limine
	mkdir -p iso_root/EFI/BOOT
	
	# Copy the kernel from the cargo target directory
	cp $(KERNEL_ELF) iso_root/boot/kernel
	cp build_deps/ramdisk.tar iso_root/boot/ramdisk.tar
	cp build_deps/limine.conf iso_root/boot/limine/
	
	# x86_64 Specific Limine binaries
	cp build_deps/limine/limine-bios.sys build_deps/limine/limine-bios-cd.bin build_deps/limine/limine-uefi-cd.bin iso_root/boot/limine/
	cp build_deps/limine/BOOTX64.EFI iso_root/EFI/BOOT/
	cp build_deps/limine/BOOTIA32.EFI iso_root/EFI/BOOT/
	
	xorriso -report_about FAILURE -as mkisofs -b boot/limine/limine-bios-cd.bin \
		-no-emul-boot -boot-load-size 4 -boot-info-table \
		--efi-boot boot/limine/limine-uefi-cd.bin \
		-efi-boot-part --efi-boot-image --protective-msdos-label \
		iso_root -o target/build/$(IMAGE_NAME).iso
	
	./build_deps/limine/limine bios-install target/build/$(IMAGE_NAME).iso
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
	rm -rf iso_root target/build

.PHONY: distclean
distclean: clean
	rm -rf build_deps/limine build_deps/edk2-ovmf
