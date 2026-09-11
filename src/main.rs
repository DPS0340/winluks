use clap::{Parser, Subcommand};
use std::{path::PathBuf, process::ExitCode};
#[cfg(windows)]
use winluks::volume::UnlockedVolume;
use winluks::{Error, Result, image::Image, metadata::Metadata, probe::Filesystem};
#[cfg(windows)]
use zeroize::Zeroizing;
#[derive(Parser)]
#[command(
    version,
    about = "Experimental read-only LUKS2 partition-image bridge",
    long_about = "Experimental read-only LUKS2 partition-image bridge.\n\nWinSpd - Windows Storage Proxy Driver, Copyright (C) Bill Zissimopoulos.\nhttps://github.com/winfsp/winspd"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    Inspect {
        #[arg(long)]
        image: PathBuf,
    },
    Open {
        #[arg(long)]
        image: PathBuf,
        #[arg(long)]
        keyslot: u32,
        #[arg(long, value_enum)]
        filesystem: Filesystem,
        #[arg(long, required = true)]
        read_only: bool,
    },
}
fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Inspect { image } => {
            let image = Image::open(&image)?;
            let m = Metadata::read(&image)?;
            println!(
                "{}",
                serde_json::json!({"format":"LUKS2","profile":"v0.2","filesystem":"unknown_locked","sector_size":512,"volume_bytes":m.volume_length(),"keyslots":m.keyslots(),"read_only":true})
            );
            Ok(())
        }
        Command::Open {
            image,
            keyslot,
            filesystem,
            read_only: _,
        } => {
            let image = Image::open(&image)?;
            let m = Metadata::read(&image)?;
            if !m.keyslots().contains(&keyslot) {
                return Err(Error::UnsupportedProfile);
            }
            #[cfg(not(windows))]
            {
                let _ = (image, filesystem);
                Err(Error::FsDriverUnavailable)
            }
            #[cfg(windows)]
            {
                use std::io::IsTerminal;
                winluks::adapter::check_consumer(filesystem)?;
                if !std::io::stdin().is_terminal() {
                    return Err(Error::ConsoleRequired);
                }
                let password = Zeroizing::new(
                    rpassword::prompt_password("Password: ").map_err(|_| Error::ConsoleRequired)?,
                );
                let v = UnlockedVolume::unlock(image, keyslot, password.as_bytes())?;
                drop(password);
                if !v.key_is_locked() {
                    eprintln!("KEY_MEMORY_LOCK_UNAVAILABLE");
                }
                let v = v.validate(filesystem)?;
                eprintln!("FS_PROFILE_VALID read_only=true");
                winluks::adapter::serve(v)
            }
        }
    }
}
fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
