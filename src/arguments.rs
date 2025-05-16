use std::num::NonZeroU8;

use anyhow::{Context, Error, Result, anyhow, ensure};
use gumdrop::{Options, ParsingStyle};

fn parse_hex(value: &str) -> Result<u8> {
    const PREFIX: &str = "0x";
    let value = value
        .strip_prefix(PREFIX)
        .ok_or_else(|| anyhow!("strip prefix {PREFIX}"))?;
    u8::from_str_radix(value, 16).context("from str radix")
}

#[derive(Debug, gumdrop::Options)]
struct Raw {
    /// print help message
    help: bool,

    /// system I/O port to which guest will write when it's ready for exit signals
    #[options(parse(try_from_str = "parse_hex"))]
    ready_for_exit_signal_port: Option<u8>,

    /// system I/O port to which guest will write exit status to request VM poweroff
    #[options(parse(try_from_str = "parse_hex"))]
    exit_port: Option<u8>,

    /// status code expected from QEMU on successful exit
    exit_code: Option<NonZeroU8>,

    /// guest architecture (eg: "x86_64") followed by any QEMU arguments
    #[options(free)]
    guest_architecture_then_qemu_arguments: Vec<String>,
}

pub struct Arguments {
    pub ready_for_exit_signal_port: u8,
    pub exit_port: u8,
    pub exit_code: i32,
    pub guest_architecture: String,
    pub qemu_arguments: Vec<String>,
}

impl Arguments {
    fn usage(argv0: &Option<impl AsRef<str>>, usage: &str) -> Error {
        let argv0 = argv0
            .as_ref()
            .map(|argv0| argv0.as_ref().to_owned())
            .unwrap_or_default();
        anyhow!("Usage: {argv0} [OPTIONS]\n\n{usage}")
    }

    pub fn parse(arguments: impl IntoIterator<Item = impl AsRef<str>>) -> Result<Self> {
        let mut arguments = arguments.into_iter();
        let argv0 = arguments.next();
        let options = Raw::parse_args(
            &arguments.collect::<Vec<_>>(),
            ParsingStyle::StopAtFirstFree,
        )
        .context("parse options")?;

        let usage = options.self_usage();
        if options.help_requested() {
            return Err(Self::usage(&argv0, usage));
        }

        let Raw {
            help: _,
            ready_for_exit_signal_port,
            exit_port,
            exit_code,
            guest_architecture_then_qemu_arguments,
        } = options;

        let mut guest_architecture_then_qemu_arguments =
            guest_architecture_then_qemu_arguments.into_iter();
        let guest_architecture =
            guest_architecture_then_qemu_arguments
                .next()
                .ok_or_else(|| {
                    Self::usage(&argv0, usage).context("missing <guest_architecture> argument")
                })?;

        let ready_for_exit_signal_port = ready_for_exit_signal_port.ok_or_else(|| {
            Self::usage(&argv0, usage).context("missing --ready-for-exit-signal-port argument")
        })?;

        let exit_port = exit_port
            .ok_or_else(|| Self::usage(&argv0, usage).context("missing --exit-port argument"))?;

        ensure!(
            ready_for_exit_signal_port != exit_port,
            "--ready-for-exit-signal-port and --exit-port arguments must be different"
        );

        Ok(Self {
            ready_for_exit_signal_port,
            exit_port,
            exit_code: exit_code
                .ok_or_else(|| Self::usage(&argv0, usage).context("missing --exit-code argument"))?
                .get()
                .into(),
            guest_architecture,
            qemu_arguments: guest_architecture_then_qemu_arguments.collect(),
        })
    }
}
