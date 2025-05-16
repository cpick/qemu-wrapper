use anyhow::{Context, Error, Result, anyhow};
use gumdrop::{Options, ParsingStyle};

#[derive(Debug, gumdrop::Options)]
struct Raw {
    /// print help message
    help: bool,

    /// guest architecture (eg: "x86_64") followed by any QEMU arguments
    #[options(free)]
    guest_architecture_then_qemu_arguments: Vec<String>,
}

pub struct Arguments {
    pub guest_architecture: String,
    pub qemu_arguments: Vec<String>,
}

impl Arguments {
    fn usage(argv0: Option<impl AsRef<str>>, usage: &str) -> Error {
        let argv0 = argv0
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
            return Err(Self::usage(argv0, usage));
        }

        let Raw {
            help: _,
            guest_architecture_then_qemu_arguments,
        } = options;

        let mut guest_architecture_then_qemu_arguments =
            guest_architecture_then_qemu_arguments.into_iter();
        let guest_architecture =
            guest_architecture_then_qemu_arguments
                .next()
                .ok_or_else(|| {
                    Self::usage(argv0, usage).context("missing <guest_architecture> argument")
                })?;

        Ok(Self {
            guest_architecture,
            qemu_arguments: guest_architecture_then_qemu_arguments.collect(),
        })
    }
}
