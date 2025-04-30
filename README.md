# qemu-plugin-ready

A QEMU plugin that allows the guest to notify the host that it's "ready".

The guest signals that it's ready by using the `out` instruction to write (any) 1-byte value to the
port 0xf5 addressed as a imm8 immediate.  That is: executing the following
[x86/x86_64 opcode](https://www.felixcloutier.com/x86/out): `[0xe6, 0xf5]`.
