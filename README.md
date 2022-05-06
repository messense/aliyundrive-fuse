# aliyundrive-fuse

[![GitHub Actions](https://github.com/messense/aliyundrive-fuse/workflows/CI/badge.svg)](https://github.com/messense/aliyundrive-fuse/actions?query=workflow%3ACI)
[![PyPI](https://img.shields.io/pypi/v/aliyundrive-fuse.svg)](https://pypi.org/project/aliyundrive-fuse)
[![Docker Image](https://img.shields.io/docker/pulls/messense/aliyundrive-fuse.svg?maxAge=2592000)](https://hub.docker.com/r/messense/aliyundrive-fuse/)
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

### OpenWrt 路由器

[GitHub Releases](https://github.com/messense/aliyundrive-fuse/releases) 中有预编译的 ipk 文件， 目前提供了
aarch64/arm/x86_64/i686 等架构的版本，可以下载后使用 opkg 安装，以 nanopi r4s 为例：

```bash
wget https://github.com/messense/aliyundrive-fuse/releases/download/v0.1.11/aliyundrive-fuse_0.1.11-1_aarch64_generic.ipk
wget https://github.com/messense/aliyundrive-fuse/releases/download/v0.1.11/luci-app-aliyundrive-fuse_0.1.11_all.ipk
wget https://github.com/messense/aliyundrive-fuse/releases/download/v0.1.11/luci-i18n-aliyundrive-fuse-zh-cn_0.1.11-1_all.ipk
opkg install aliyundrive-fuse_0.1.11-1_aarch64_generic.ipk
opkg install luci-app-aliyundrive-fuse_0.1.11_all.ipk
opkg install luci-i18n-aliyundrive-fuse-zh-cn_0.1.11-1_all.ipk
```

其它 CPU 架构的路由器可在 [GitHub Releases](https://github.com/messense/aliyundrive-fuse/releases) 页面中查找对应的架构的主程序 ipk 文件下载安装。

> Tips: 不清楚 CPU 架构类型可通过运行 `opkg print-architecture` 命令查询。

## 命令行用法

```bash
USAGE:
    aliyundrive-fuse [OPTIONS] --refresh-token <REFRESH_TOKEN> <PATH>

ARGS:
    <PATH>    Mount point

OPTIONS:
        --allow-other                            Allow other users to access the drive
        --domain-id <DOMAIN_ID>                  Aliyun PDS domain id
    -h, --help                                   Print help information
    -r, --refresh-token <REFRESH_TOKEN>          Aliyun drive refresh token [env: REFRESH_TOKEN=]
    -S, --read-buffer-size <READ_BUFFER_SIZE>    Read/download buffer size in bytes, defaults to 10MB [default: 10485760]
    -V, --version                                Print version information
    -w, --workdir <WORKDIR>                      Working directory, refresh_token will be stored in there if specified
```

比如将磁盘挂载到 `/mnt/aliyundrive` 目录：

```bash
mkdir -p /mnt/aliyundrive /var/run/aliyundrive-fuse
aliyundrive-fuse -r your-refresh-token -w /var/run/aliyundrive-fuse /mnt/aliyundrive
```

## Emby/Jellyfin

如果是直接运行在系统上的 Emby/Jellyfin，则可以直接在其控制台添加媒体库的时候选择阿里云盘对应的挂载路径中的文件夹即可；
如果是 Docker 运行的 Emby/Jellyfin，则需要将阿里云盘挂载路径也挂载到 Docker 容器中，假设阿里云盘挂载路径为 `/mnt/aliyundrive`，
以 Jellyfin 为例（假设 Jellyfin 工作路径为 `/root/jellyfin`）将云盘挂载到容器 `/media` 路径：

```bash
docker run -d --name jellyfin \
  -v /root/jellyfin/config:/config \
  -v /root/jellyfin/cache:/cache \
  -v /mnt/aliyundrive:/media \
  -p 8096:8096 \
  --device=/dev/dri/renderD128 \
  --device /dev/dri/card0:/dev/dri/card0 \
  --restart unless-stopped \
  jellyfin/jellyfin
```

## License

This work is released under the MIT license. A copy of the license is provided in the [LICENSE](./LICENSE) file.
