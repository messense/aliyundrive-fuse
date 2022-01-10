# aliyundrive-fuse

[![GitHub Actions](https://github.com/messense/aliyundrive-fuse/workflows/CI/badge.svg)](https://github.com/messense/aliyundrive-fuse/actions?query=workflow%3ACI)
[![PyPI](https://img.shields.io/pypi/v/aliyundrive-fuse.svg)](https://pypi.org/project/aliyundrive-fuse)
[![aliyundrive-fuse](https://snapcraft.io/aliyundrive-fuse/badge.svg)](https://snapcraft.io/aliyundrive-fuse)
[![Crates.io](https://img.shields.io/crates/v/aliyundrive-fuse.svg)](https://crates.io/crates/aliyundrive-fuse)

> 🚀 Help me to become a full-time open-source developer by [sponsoring me on GitHub](https://github.com/sponsors/messense)

阿里云盘 FUSE 磁盘挂载，主要用于配合 [Emby](https://emby.media) 或者 [Jellyfin](https://jellyfin.org) 观看阿里云盘内容，功能特性：

1. 目前只读，不支持写入
2. 支持 Linux 和 macOS，暂不支持 Windows

[aliyundrive-webdav](https://github.com/messense/aliyundrive-webdav) 项目已经实现了通过 WebDAV 访问阿里云盘内容，但由于 Emby 和 Jellyfin 都不支持直接访问 WebDAV 资源，
需要配合 [rclone](https://rclone.org) 之类的软件将 WebDAV 挂载为本地磁盘，而本项目则直接通过 FUSE 实现将阿里云盘挂载为本地磁盘，省去使用 rclone 再做一层中转。

## 安装

* macOS 需要先安装 [macfuse](https://osxfuse.github.io/)
* Linux 需要先安装 fuse
  * Debian 系如 Ubuntu: `apt-get install -y fuse3`
  * RedHat 系如 CentOS: `yum install -y fuse3`

可以从 [GitHub Releases](https://github.com/messense/aliyundrive-fuse/releases) 页面下载预先构建的二进制包， 也可以使用 pip 从 PyPI 下载:

```bash
pip install aliyundrive-fuse
```

如果系统支持 [Snapcraft](https://snapcraft.io) 比如 Ubuntu、Debian 等，也可以使用 snap 安装：

```bash
sudo snap install aliyundrive-fuse
```

## 命令行用法

```bash
USAGE:
    aliyundrive-fuse [OPTIONS] --refresh-token <REFRESH_TOKEN> <PATH>

ARGS:
    <PATH>    Mount point

OPTIONS:
        --domain-id <DOMAIN_ID>            Aliyun PDS domain id
    -h, --help                             Print help information
    -r, --refresh-token <REFRESH_TOKEN>    Aliyun drive refresh token [env: REFRESH_TOKEN=]
    -V, --version                          Print version information
    -w, --workdir <WORKDIR>                Working directory, refresh_token will be stored in there if specified
```

比如将磁盘挂载到 `/mnt/aliyundrive` 目录：

```bash
mkdir -p /mnt/aliyundrive /var/run/aliyundrive-fuse
aliyundrive-fuse -r your-refresh-token -w /var/run/aliyundrive-fuse /mnt/aliyundrive
```

## License

This work is released under the MIT license. A copy of the license is provided in the [LICENSE](./LICENSE) file.
