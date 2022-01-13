use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use bytes::Bytes;
use parking_lot::RwLock;
use reqwest::{
    header::{HeaderMap, HeaderValue},
    StatusCode,
};
use serde::de::DeserializeOwned;
use serde::Serialize;
use tracing::{debug, error, info, warn};

mod model;

use model::*;
pub use model::{AliyunFile, DateTime, FileType};

const ORIGIN: &str = "https://www.aliyundrive.com";
const REFERER: &str = "https://www.aliyundrive.com/";
const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/92.0.4515.131 Safari/537.36";

#[derive(Debug, Clone)]
pub struct DriveConfig {
    pub api_base_url: String,
    pub refresh_token_url: String,
    pub workdir: Option<PathBuf>,
    pub app_id: Option<String>,
}

#[derive(Debug, Clone)]
struct Credentials {
    refresh_token: String,
    access_token: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AliyunDrive {
    config: DriveConfig,
    client: reqwest::blocking::Client,
    credentials: Arc<RwLock<Credentials>>,
    drive_id: Option<String>,
    pub nick_name: Option<String>,
}

impl AliyunDrive {
    pub fn new(config: DriveConfig, refresh_token: String) -> Result<Self> {
        let credentials = Credentials {
            refresh_token,
            access_token: None,
        };
        let mut headers = HeaderMap::new();
        headers.insert("Origin", HeaderValue::from_static(ORIGIN));
        headers.insert("Referer", HeaderValue::from_static(REFERER));
        let client = reqwest::blocking::Client::builder()
            .user_agent(UA)
            .default_headers(headers)
            // OSS closes idle connections after 60 seconds,
            // so we can close idle connections ahead of time to prevent re-using them.
            // See also https://github.com/hyperium/hyper/issues/2136
            .pool_idle_timeout(Duration::from_secs(50))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()?;
        let mut drive = Self {
            config,
            client,
            credentials: Arc::new(RwLock::new(credentials)),
            drive_id: None,
            nick_name: None,
        };

        let (tx, rx) = oneshot::channel();
        // schedule update token task
        let client = drive.clone();
        let refresh_token_from_file = if let Some(dir) = drive.config.workdir.as_ref() {
            fs::read_to_string(dir.join("refresh_token")).ok()
        } else {
            None
        };
        thread::spawn(move || {
            let mut delay_seconds = 7000;
            match client.do_refresh_token_with_retry(refresh_token_from_file) {
                Ok(res) => {
                    // token usually expires in 7200s, refresh earlier
                    delay_seconds = res.expires_in - 200;
                    if tx.send((res.default_drive_id, res.nick_name)).is_err() {
                        error!("send default drive id failed");
                    }
                }
                Err(err) => {
                    error!("refresh token failed: {}", err);
                    tx.send((String::new(), String::new())).unwrap();
                }
            }
            loop {
                thread::sleep(Duration::from_secs(delay_seconds));
                if let Err(err) = client.do_refresh_token_with_retry(None) {
                    error!("refresh token failed: {}", err);
                }
            }
        });

        let (drive_id, nick_name) = rx.recv()?;
        if drive_id.is_empty() {
            bail!("get default drive id failed");
        }
        info!(drive_id = %drive_id, "found default drive");
        drive.drive_id = Some(drive_id);
        drive.nick_name = Some(nick_name);

        Ok(drive)
    }

    fn save_refresh_token(&self, refresh_token: &str) -> Result<()> {
        if let Some(dir) = self.config.workdir.as_ref() {
            fs::create_dir_all(dir)?;
            let refresh_token_file = dir.join("refresh_token");
            fs::write(refresh_token_file, refresh_token)?;
        }
        Ok(())
    }

    fn do_refresh_token(&self, refresh_token: &str) -> Result<RefreshTokenResponse> {
        let mut data = HashMap::new();
        data.insert("refresh_token", refresh_token);
        data.insert("grant_type", "refresh_token");
        if let Some(app_id) = self.config.app_id.as_ref() {
            data.insert("app_id", app_id);
        }
        let res = self
            .client
            .post(&self.config.refresh_token_url)
            .json(&data)
            .send()?;
        match res.error_for_status_ref() {
            Ok(_) => {
                let res = res.json::<RefreshTokenResponse>()?;
                info!(
                    refresh_token = %res.refresh_token,
                    nick_name = %res.nick_name,
                    "refresh token succeed"
                );
                Ok(res)
            }
            Err(err) => {
                let msg = res.text()?;
                let context = format!("{}: {}", err, msg);
                Err(err).context(context)
            }
        }
    }

    fn do_refresh_token_with_retry(
        &self,
        refresh_token_from_file: Option<String>,
    ) -> Result<RefreshTokenResponse> {
        let mut last_err = None;
        let mut refresh_token = self.refresh_token();
        for _ in 0..10 {
            match self.do_refresh_token(&refresh_token) {
                Ok(res) => {
                    let mut cred = self.credentials.write();
                    cred.refresh_token = res.refresh_token.clone();
                    cred.access_token = Some(res.access_token.clone());
                    if let Err(err) = self.save_refresh_token(&res.refresh_token) {
                        error!(error = %err, "save refresh token failed");
                    }
                    return Ok(res);
                }
                Err(err) => {
                    let mut should_warn = true;
                    let mut should_retry = match err.downcast_ref::<reqwest::Error>() {
                        Some(e) => {
                            e.is_connect()
                                || e.is_timeout()
                                || matches!(e.status(), Some(StatusCode::TOO_MANY_REQUESTS))
                        }
                        None => false,
                    };
                    // retry if command line refresh_token is invalid but we also have
                    // refresh_token from file
                    if let Some(refresh_token_from_file) = refresh_token_from_file.as_ref() {
                        if !should_retry && &refresh_token != refresh_token_from_file {
                            refresh_token = refresh_token_from_file.trim().to_string();
                            should_retry = true;
                            // don't warn if we are gonna try refresh_token from file
                            should_warn = false;
                        }
                    }
                    if should_retry {
                        if should_warn {
                            warn!(error = %err, "refresh token failed, will wait and retry");
                        }
                        last_err = Some(err);
                        thread::sleep(Duration::from_secs(1));
                        continue;
                    } else {
                        last_err = Some(err);
                        break;
                    }
                }
            }
        }
        Err(last_err.unwrap())
    }

    fn refresh_token(&self) -> String {
        let cred = self.credentials.read();
        cred.refresh_token.clone()
    }

    fn access_token(&self) -> Result<String> {
        let cred = self.credentials.read();
        cred.access_token.clone().context("missing access_token")
    }

    fn drive_id(&self) -> Result<&str> {
        self.drive_id.as_deref().context("missing drive_id")
    }

    fn request<T, U>(&self, url: String, req: &T) -> Result<Option<U>>
    where
        T: Serialize + ?Sized,
        U: DeserializeOwned,
    {
        let mut access_token = self.access_token()?;
        let url = reqwest::Url::parse(&url)?;
        let res = self
            .client
            .post(url.clone())
            .bearer_auth(&access_token)
            .json(&req)
            .send()?
            .error_for_status();
        match res {
            Ok(res) => {
                if res.status() == StatusCode::NO_CONTENT {
                    return Ok(None);
                }
                let res = res.json::<U>()?;
                Ok(Some(res))
            }
            Err(err) => {
                match err.status() {
                    Some(
                        status_code
                        @
                        // 4xx
                        (StatusCode::UNAUTHORIZED
                        | StatusCode::REQUEST_TIMEOUT
                        | StatusCode::TOO_MANY_REQUESTS
                        // 5xx
                        | StatusCode::INTERNAL_SERVER_ERROR
                        | StatusCode::BAD_GATEWAY
                        | StatusCode::SERVICE_UNAVAILABLE
                        | StatusCode::GATEWAY_TIMEOUT),
                    ) => {
                        if status_code == StatusCode::UNAUTHORIZED {
                            // refresh token and retry
                            let token_res = self.do_refresh_token_with_retry(None)?;
                            access_token = token_res.access_token;
                        } else {
                            // wait for a while and retry
                            thread::sleep(Duration::from_secs(1));
                        }
                        let res = self
                            .client
                            .post(url)
                            .bearer_auth(&access_token)
                            .json(&req)
                            .send()
                            ?
                            .error_for_status()?;
                        if res.status() == StatusCode::NO_CONTENT {
                            return Ok(None);
                        }
                        let res = res.json::<U>()?;
                        Ok(Some(res))
                    }
                    _ => Err(err.into()),
                }
            }
        }
    }

    pub fn list_all(&self, parent_file_id: &str) -> Result<Vec<AliyunFile>> {
        let mut files = Vec::new();
        let mut marker = None;
        loop {
            let res = self.list(parent_file_id, marker.as_deref())?;
            files.extend(res.items.into_iter());
            if res.next_marker.is_empty() {
                break;
            }
            marker = Some(res.next_marker);
        }
        Ok(files)
    }

    pub fn list(&self, parent_file_id: &str, marker: Option<&str>) -> Result<ListFileResponse> {
        let drive_id = self.drive_id()?;
        debug!(drive_id = %drive_id, parent_file_id = %parent_file_id, marker = ?marker, "list file");
        let req = ListFileRequest {
            drive_id,
            parent_file_id,
            limit: 200,
            all: false,
            image_thumbnail_process: "image/resize,w_400/format,jpeg",
            image_url_process: "image/resize,w_1920/format,jpeg",
            video_thumbnail_process: "video/snapshot,t_0,f_jpg,ar_auto,w_300",
            fields: "*",
            order_by: "updated_at",
            order_direction: "DESC",
            marker,
        };
        self.request(format!("{}/v2/file/list", self.config.api_base_url), &req)
            .and_then(|res| res.context("expect response"))
    }

    pub fn download(&self, url: &str, start_pos: u64, size: usize) -> Result<Bytes> {
        use reqwest::header::RANGE;

        let end_pos = start_pos + size as u64 - 1;
        debug!(url = %url, start = start_pos, end = end_pos, "download file");
        let range = format!("bytes={}-{}", start_pos, end_pos);
        let res = self
            .client
            .get(url)
            .header(RANGE, range)
            .send()?
            .error_for_status()?;
        Ok(res.bytes()?)
    }

    pub fn get_download_url(&self, file_id: &str) -> Result<String> {
        debug!(file_id = %file_id, "get download url");
        let req = GetFileDownloadUrlRequest {
            drive_id: self.drive_id()?,
            file_id,
        };
        let res: GetFileDownloadUrlResponse = self
            .request(
                format!("{}/v2/file/get_download_url", self.config.api_base_url),
                &req,
            )?
            .context("expect response")?;
        Ok(res.url)
    }

    pub fn get_quota(&self) -> Result<(u64, u64)> {
        let drive_id = self.drive_id()?;
        let mut data = HashMap::new();
        data.insert("drive_id", drive_id);
        let res: GetDriveResponse = self
            .request(format!("{}/v2/drive/get", self.config.api_base_url), &data)?
            .context("expect response")?;
        Ok((res.used_size, res.total_size))
    }
}
