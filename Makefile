#
# Global Makefile
# Author: hkdywg <hkdywg@163.com>
#
# Copyright (C) 2026 kernel
#
#

.PHONY: all run clean

all:
	cargo build --target aarch64-unknown-none --release --bin kernel

run:
	qemu-system-aarch64 -M virt -m 256 -cpu cortex-a53 -kernel target/aarch64-unknown-none/release/kernel -nographic

debug:
	qemu-system-aarch64 -M raspi3 -kernel target/aarch64-unknown-none/debug/kernel -s -S &
	gdb-multiarch target/aarch64-unknown-none/debug/kernel

clean:
	cargo clean
