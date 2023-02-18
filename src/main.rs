use std::{env, io, path::PathBuf};

use clap::Parser;
use fuser::MountOption;

use drive::{AliyunDrive, DriveConfig};
use vfs::AliyunDriveFileSystem;

mod drive;
mod error;
mod file_cache;
mod vfs;

#[derive(Parser, Debug)]
#[command(name = "aliyundrive-fuse", about, version, author)]
struct Opt {
    /// Mount point
    #[arg(long)]
    path: PathBuf,
    /// Aliyun drive refresh token
    #[arg(short, long, env = "REFRESH_TOKEN")]
    refresh_token: String,
    /// Working directory, refresh_token will be stored in there if specified
    #[arg(short = 'w', long)]
    workdir: Option<PathBuf>,
    /// Allow other users to access the drive
    #[arg(long)]
    allow_other: bool,
    /// Read/download buffer size in bytes, defaults to 10MB
    #[arg(short = 'S', long, default_value = "10485760")]
    read_buffer_size: usize,
}

fn main() -> anyhow::Result<()> {
    #[cfg(feature = "native-tls-vendored")]
    openssl_probe::init_ssl_cert_env_vars();

    if env::var("RUST_LOG").is_err() {
        env::set_var("RUST_LOG", "aliyundrive_fuse=info");
    }
    tracing_subscriber::fmt::init();

    let opt = Opt::parse();
    let drive_config = DriveConfig {
        api_base_url: "https://api.aliyundrive.com".to_string(),
        refresh_token_url: "https://api.aliyundrive.com/token/refresh".to_string(),
        workdir: opt.workdir,
        app_id: None,
    };
    let drive = AliyunDrive::new(drive_config, opt.refresh_token).map_err(|_| {
        io::Error::new(io::ErrorKind::Other, "initialize aliyundrive client failed")
    })?;

    let _nick_name = drive.nick_name.clone();
    let vfs = AliyunDriveFileSystem::new(drive, opt.read_buffer_size);
    let mut mount_options = vec![MountOption::AutoUnmount, MountOption::NoAtime];
    // read only for now
    mount_options.push(MountOption::RO);
    if opt.allow_other {
        mount_options.push(MountOption::AllowOther);
    }
    if cfg!(target_os = "macos") {
        mount_options.push(MountOption::CUSTOM("local".to_string()));
        mount_options.push(MountOption::CUSTOM("noappledouble".to_string()));
        let volname = if let Some(nick_name) = _nick_name {
            format!("volname=阿里云盘({})", nick_name)
        } else {
            "volname=阿里云盘".to_string()
        };
        mount_options.push(MountOption::CUSTOM(volname));
    }
    fuser::mount2(vfs, opt.path, &mount_options)?;
    Ok(())
}
