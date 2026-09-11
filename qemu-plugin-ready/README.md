# qemu-plugin-ready

A QEMU plugin that allows the guest to notify the host that it's "ready".

## Usage

Build plugin:
```sh
cargo build --release
```

In a parent/supervisor process, open a a `pipe()`, ensure its sender file descriptor is not marked
close-on-exec, `fork()`, and have the child process `exec()` QEMU with the following option (replace
".dylib" with ".so" or ".dll" as appropriate for host operating system):
```sh
qemu-system-x86_64 -plugin target/release/libqemu_plugin_ready.dylib,port=0xf5,fd=<PIPE_SENDER_FD>
```
(The `-d plugin` option can be added to see a log message when the child is ready.) 

The guest signals that it's ready by using the `out` instruction to write (any) 1-byte value to the
port 0xf5 addressed as a imm8 immediate.  That is: executing the following
[x86/x86_64 opcode](https://www.felixcloutier.com/x86/out): `[0xe6, 0xf5]`.

The host QEMU's plugin detects this and closes the configured file descriptor.

The parent/supervisor process can wait for this by `poll()`/`select()`ing on the pipe's receiver
file descriptor to see when the sender has been closed, indicating that the VM is "ready".
