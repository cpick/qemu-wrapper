# qemu-wrapper

Wrap QEMU and convert a SIGINT (Ctrl+C) into a `system_poweroff`
[monitor command](https://www.qemu.org/docs/master/system/monitor.html#commands).

Without a preceeding `system_poweroff`, QEMU will flush its disks and halt the machine without
notifying the operating system.
