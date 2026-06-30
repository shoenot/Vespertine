# Nuke built-in rules and variables.
MAKEFLAGS += -rR --silent
.SUFFIXES:

TARGET_NAME := x86_64-unknown-none
AS          := nasm
OBJ_DIR     := ../target/build

.PHONY: all
all: $(OBJ_DIR)/syscall.o
	echo "[INFO] Building kernel"
	cd .. && cargo build -p kernel --release --target $(TARGET_NAME)

$(OBJ_DIR)/gdt.o: ../lib/vk-hal/src/x86_64/cpu/gdt.asm
	mkdir -p $(OBJ_DIR)
	$(AS) -f elf64 $< -o $@

$(OBJ_DIR)/idt.o: src/arch/x86_64/interrupts/idt.asm $(OBJ_DIR)/gdt.o
	mkdir -p $(OBJ_DIR)
	$(AS) -f elf64 $< -o $@

$(OBJ_DIR)/switch.o: ../lib/vk-hal/src/x86_64/task/switch.asm $(OBJ_DIR)/idt.o
	mkdir -p $(OBJ_DIR)
	$(AS) -f elf64 $< -o $@

$(OBJ_DIR)/fpu.o: ../lib/vk-hal/src/x86_64/fpu/fpu.asm $(OBJ_DIR)/switch.o
	mkdir -p $(OBJ_DIR)
	$(AS) -f elf64 $< -o $@

$(OBJ_DIR)/syscall.o: src/arch/x86_64/task/syscall.asm $(OBJ_DIR)/fpu.o
	mkdir -p $(OBJ_DIR)
	$(AS) -f elf64 $< -o $@

.PHONY: clean
clean:
	rm -f $(OBJ_DIR)/*.o
	cargo clean -p kernel
