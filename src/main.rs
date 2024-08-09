mod mem_probe;
mod opendal_fs;

use clap::Parser;
use fs_extra::dir;
use mem_probe::MemProbe;
use opendal::{layers::MimeGuessLayer, services::Oss, Operator};
use opendal_fs::{sync_dir, ConcurrentUploadTasks};
use pushover_rs::{send_pushover_request, PushoverSound};
use serde::Deserialize;
use std::{
    env::{self, current_exe, set_current_dir},
    ffi::{OsStr, OsString},
    io::Read,
    path::{Path, PathBuf},
};
use tokio::{
    fs::{self, remove_dir_all},
    process::Command,
    task::spawn_blocking,
};
use tracing_subscriber::fmt::{format::FmtSpan, time::ChronoLocal};

#[derive(Parser, Debug)]
enum Commands {
    Start,
    UpgradeHugo,
    Run,
}

impl Commands {
    fn init() -> Self {
        let s = Self::parse();
        match s {
            Self::Start => tracing::info!("workflow-bin start"),
            Self::UpgradeHugo => tracing::info!("workflow-bin upgrade-hugo"),
            Self::Run => tracing::info!("workflow-bin run"),
        }
        s
    }

    fn is_start(&self) -> bool {
        if let Self::Start = self {
            true
        } else {
            false
        }
    }

    fn is_run(&self) -> bool {
        if let Self::Run = self {
            true
        } else {
            false
        }
    }
}

impl Drop for Commands {
    fn drop(&mut self) {
        tracing::info!("执行完毕！");
    }
}

fn env_var(key: impl AsRef<OsStr>) -> Result<String, anyhow::Error> {
    Ok(env::var(key)?)
}

struct Pushover {
    user_key: String,
    app_token: String,
}

impl Pushover {
    fn new() -> Result<Self, anyhow::Error> {
        Ok(Self {
            user_key: env_var("PUSHOVER_USER_KEY")?,
            app_token: env_var("PUSHOVER_APP_TOKEN")?,
        })
    }

    async fn send(&self, message: &str, sound: PushoverSound) -> Result<(), anyhow::Error> {
        tracing::info!("正在发送Pushover消息：{}", message);
        tracing::info!("Pushover音色：{}", sound);

        match send_pushover_request(
            pushover_rs::MessageBuilder::new(&self.user_key, &self.app_token, message)
                .set_sound(sound)
                .build(),
        )
        .await
        {
            Ok(res) => match res.errors {
                None => Ok(()),
                Some(errs) => Err(anyhow::anyhow!("{}", errs.join("\r\n"))),
            },
            Err(err) => Err(anyhow::anyhow!("{}", err)),
        }
    }
}

#[cfg(debug_assertions)]
pub fn install_tracing() {
    tracing_subscriber::fmt()
        .with_timer(ChronoLocal::new("%m-%d %H:%M:%S".into()))
        .with_max_level(tracing::Level::DEBUG)
        .with_span_events(FmtSpan::FULL)
        .with_thread_names(true)
        .init();
}

#[cfg(not(debug_assertions))]
pub fn install_tracing() {
    tracing_subscriber::fmt()
        .with_timer(ChronoLocal::new("%m-%d %H:%M:%S".into()))
        .with_span_events(FmtSpan::FULL)
        .with_thread_names(true)
        .init();
}

#[derive(Deserialize)]
struct HugoConfig {
    version: String,
}

#[derive(Deserialize)]
struct GithubDeployConfig {
    username: String,
    org: String,
    repo: String,
    access_token: Option<String>,
    user_email: Option<String>,
    user_name: Option<String>,
}

#[derive(Deserialize)]
struct OssSyncConfig {
    root: String,
    files: Vec<String>,
    dirs: Vec<String>,
}

#[derive(Deserialize)]
struct OssDeployConfig {
    sync: OssSyncConfig,
    access_key_id: Option<String>,
    access_key_secret: Option<String>,
}

#[derive(Deserialize)]
struct DeployConfig {
    github: GithubDeployConfig,
    oss: OssDeployConfig,
}

#[derive(Deserialize)]
struct WorkflowConfig {
    hugo: HugoConfig,
    deploy: DeployConfig,
}

impl WorkflowConfig {
    async fn read() -> Result<Self, anyhow::Error> {
        tracing::info!("正在读取workflow.toml……");
        Ok(toml::from_str(&fs::read_to_string("workflow.toml").await?)?)
    }
}

fn retain_decimal_places(f: f64, n: i32) -> f64 {
    let power = 10.0f64.powi(n);
    (f * power).round() / power
}

#[cfg(windows)]
fn unzip(z: &[u8]) -> Result<(OsString, Vec<u8>), anyhow::Error> {
    use std::io::Cursor;
    use zip::ZipArchive;

    let mut archive = ZipArchive::new(Cursor::new(z))?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let path = file
            .enclosed_name()
            .ok_or(anyhow::anyhow!("压缩文件路径异常！"))?;
        let name = path
            .file_name()
            .ok_or(anyhow::anyhow!("压缩文件名异常！"))?;

        if name
            .to_str()
            .ok_or(anyhow::anyhow!("压缩文件名编码异常！"))?
            .starts_with("hugo")
        {
            let mut contents = Vec::new();
            file.read_to_end(&mut contents)?;
            return Ok((name.to_owned(), contents));
        }
    }

    Err(anyhow::anyhow!("压缩包中未找到hugo执行文件！"))
}

#[cfg(not(windows))]
fn unzip(z: &[u8]) -> Result<(OsString, Vec<u8>), anyhow::Error> {
    use flate2::read::GzDecoder;
    use tar::Archive;

    for entry in Archive::new(GzDecoder::new(z)).entries()? {
        let mut file = entry?;
        let path = file.path()?.into_owned();
        let name = path
            .file_name()
            .ok_or(anyhow::anyhow!("压缩文件名异常！"))?;

        if name
            .to_str()
            .ok_or(anyhow::anyhow!("压缩文件名编码异常！"))?
            .starts_with("hugo")
        {
            let mut contents = Vec::new();
            file.read_to_end(&mut contents)?;
            return Ok((name.to_owned(), contents));
        }
    }

    Err(anyhow::anyhow!("压缩包中未找到hugo执行文件！"))
}

#[cfg(not(windows))]
async fn chmod_exec(path: impl AsRef<std::path::Path>) -> Result<(), anyhow::Error> {
    tracing::info!("正在设置执行权限……");
    use std::{fs::Permissions, os::unix::prelude::PermissionsExt};
    Ok(fs::set_permissions(path, Permissions::from_mode(0o755)).await?)
}

async fn fetch_hugo(config: HugoConfig) -> Result<PathBuf, anyhow::Error> {
    let version = config.version;

    tracing::info!("请求的hugo版本是：{}", version);
    tracing::info!("正在校验现有hugo版本……");

    let exe = current_exe()?;
    let hugo = exe.with_file_name("hugo");
    let mut need_fetch = true;

    if let Ok(output) = Command::new(&hugo).arg("version").output().await {
        let status = output.status;

        if status.success() {
            if output
                .stdout
                .starts_with(format!("hugo v{}", version).as_bytes())
            {
                need_fetch = false;
                tracing::info!("现有hugo版本匹配！将跳过下载");
            } else {
                tracing::info!("现有hug版本不匹配，准备更新hugo");
            }
        } else {
            return Err(anyhow::anyhow!(
                "hugo version执行失败！退出码：{}",
                if let Some(code) = status.code() {
                    code.to_string()
                } else {
                    "None".into()
                }
            ));
        }
    } else {
        tracing::info!("hugo不存在，准备下载hugo");
    }

    if need_fetch {
        #[cfg(target_os = "macos")]
        const SUFFIX: &str = "darwin-universal.tar.gz";
        #[cfg(target_os = "linux")]
        const SUFFIX: &str = "Linux-64bit.tar.gz";
        #[cfg(target_os = "windows")]
        const SUFFIX: &str = "windows-amd64.zip";

        let url = format!(
            "https://github.com/gohugoio/hugo/releases/download/v{}/hugo_extended_{}_{}",
            version, version, SUFFIX
        );
        tracing::info!("正在GET：{}", url);

        let bytes = reqwest::get(url).await?.error_for_status()?.bytes().await?;

        if bytes.is_empty() {
            return Err(anyhow::anyhow!("未下载任何内容！"));
        } else {
            tracing::info!(
                "已下载：{} MB",
                retain_decimal_places(bytes.len() as f64 / 1024.0 / 1024.0, 3)
            );
            tracing::info!("正在解压……");

            let (name, contents) = unzip(&bytes)?;
            tracing::info!(
                "正在保存：{:?}（{} MB）",
                name,
                retain_decimal_places(contents.len() as f64 / 1024.0 / 1024.0, 3)
            );

            let path = exe.with_file_name(name);
            fs::write(&path, contents).await?;

            #[cfg(not(windows))]
            chmod_exec(path).await?;
        }
    }

    Ok(hugo)
}

async fn spawn_command(cmd: &mut Command, hint: &str) -> Result<(), anyhow::Error> {
    let status = cmd.spawn()?.wait().await?;

    if status.success() {
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "{}命令执行失败！退出码：{}",
            hint,
            if let Some(code) = status.code() {
                code.to_string()
            } else {
                "None".into()
            }
        ))
    }
}

async fn remove_public() -> Result<(), anyhow::Error> {
    let public = Path::new("public");
    if public.is_dir() {
        tracing::info!("正在清理public目录……");
        remove_dir_all(public).await?;
    }
    Ok(())
}

async fn copy_dir<P, Q>(from: P, to: Q) -> Result<u64, anyhow::Error>
where
    P: AsRef<Path> + Send + 'static,
    Q: AsRef<Path> + Send + 'static,
{
    Ok(spawn_blocking(move || dir::copy(from, to, &Default::default())).await??)
}

async fn deploy_github(config: &GithubDeployConfig, for_draft: bool) -> Result<(), anyhow::Error> {
    tracing::info!(
        "正在deploy github {}",
        if for_draft { "draft" } else { "main" }
    );

    let repo = &config.repo;
    let access_token = config.access_token.as_ref().unwrap();
    let url = format!(
        "https://{}:{}@github.com/{}/{}.git",
        config.username, access_token, config.org, repo
    );

    tracing::info!("正在执行：git clone {}", url.replace(access_token, "****"));
    spawn_command(Command::new("git").arg("clone").arg(url), "git").await?;
    set_current_dir(repo)?;

    tracing::info!("正在配置git环境……");
    spawn_command(
        Command::new("git")
            .arg("config")
            .arg("user.email")
            .arg(config.user_email.as_ref().unwrap()),
        "git",
    )
    .await?;
    spawn_command(
        Command::new("git")
            .arg("config")
            .arg("user.name")
            .arg(config.user_name.as_ref().unwrap()),
        "git",
    )
    .await?;

    if for_draft {
        tracing::info!("正在执行：git checkout draft");
        spawn_command(Command::new("git").arg("checkout").arg("draft"), "git").await?;
    }

    remove_public().await?;

    tracing::info!("正在拷贝public目录……");
    copy_dir("../public", "").await?;

    tracing::info!("正在提交……");
    spawn_command(Command::new("git").arg("add").arg("."), "git").await?;

    if Command::new("git")
        .arg("commit")
        .arg("-m")
        .arg("Deploy")
        .spawn()?
        .wait()
        .await?
        .success()
    {
        tracing::info!("正在执行：git push");
        spawn_command(Command::new("git").arg("push"), "git").await?;
    } else {
        tracing::warn!("没有可以提交的内容！");
    }

    tracing::info!("正在清理{}目录……", repo);
    set_current_dir("..")?;
    Ok(remove_dir_all(repo).await?)
}

async fn deploy_oss(config: &OssDeployConfig, for_draft: bool) -> Result<(), anyhow::Error> {
    tracing::info!(
        "正在deploy oss {}",
        if for_draft { "draft" } else { "prod" }
    );
    set_current_dir("public")?;

    tracing::info!("正在初始化Operator……");
    let sync = &config.sync;
    let oss = Oss::default()
        .root(&sync.root)
        .access_key_id(config.access_key_id.as_ref().unwrap())
        .access_key_secret(config.access_key_secret.as_ref().unwrap());
    let oss = if for_draft {
        oss.bucket(&env_var("OSS_DRAFT_BUCKET")?)
            .endpoint(&env_var("OSS_DRAFT_ENDPOINT")?)
    } else {
        oss.bucket(&env_var("OSS_PROD_BUCKET")?)
            .endpoint(&env_var("OSS_PROD_ENDPOINT")?)
    };

    let op = Operator::new(oss)?
        .layer(MimeGuessLayer::default())
        .finish();

    tracing::info!("开始上传文件……");
    let mut files = ConcurrentUploadTasks::new(op.clone());
    files.push_str_seq(&sync.files).await?;
    files.join().await?;

    tracing::info!("开始同步目录……");
    for dir in &sync.dirs {
        tracing::info!("正在同步目录：{}", dir);
        sync_dir(&op, dir).await?;
    }

    Ok(set_current_dir("..")?)
}

async fn hugo_deploy(
    hugo: impl AsRef<OsStr>,
    config: &DeployConfig,
    for_draft: bool,
) -> Result<(), anyhow::Error> {
    tracing::info!(
        "正在hugo deploy {}版本……",
        if for_draft { "draft" } else { "production" }
    );

    remove_public().await?;

    let mut hugo = Command::new(hugo);
    let (hugo, base_url) = if for_draft {
        let base_url = env_var("HUGO_DRAFT_BASE_URL")?;
        (
            hugo.arg("-b").arg(&base_url).arg("-D").arg("-F"),
            Some(base_url),
        )
    } else {
        (&mut hugo, None)
    };

    if let Some(base_url) = base_url {
        tracing::info!(
            "正在执行：hugo {}",
            hugo.as_std()
                .get_args()
                .collect::<Vec<&OsStr>>()
                .join(" ".as_ref())
                .to_string_lossy()
                .replace(&base_url, "****")
        );
    } else {
        tracing::info!("正在执行：hugo");
    }
    spawn_command(hugo, "hugo").await?;

    deploy_github(&config.github, for_draft).await?;
    deploy_oss(&config.oss, for_draft).await
}

trait AlertErr {
    async fn alert_err(self, trigger: bool) -> Self;
}

impl<T> AlertErr for Result<T, anyhow::Error> {
    async fn alert_err(self, trigger: bool) -> Self {
        if let Err(err) = &self {
            if trigger {
                Pushover::new()?
                    .send(
                        &format!("Workflow执行失败！原因：\r\n{}", err),
                        PushoverSound::FALLING,
                    )
                    .await?;
            }
        }
        self
    }
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    #[cfg(windows)]
    nu_ansi_term::enable_ansi_support().ok();
    install_tracing();

    let cmd = Commands::init();

    if cmd.is_start() {
        Pushover::new()?
            .send("Workflow开始执行！", PushoverSound::BIKE)
            .await
    } else {
        let config = WorkflowConfig::read().await.alert_err(cmd.is_run()).await?;
        let hugo = fetch_hugo(config.hugo)
            .await
            .alert_err(cmd.is_run())
            .await?;

        if cmd.is_run() {
            let mp = MemProbe::new();
            let mut config = config.deploy;
            config.github.access_token = Some(
                env_var("DEPLOY_GITHUB_ACCESS_TOKEN")
                    .alert_err(true)
                    .await?,
            );
            config.github.user_email =
                Some(env_var("DEPLOY_GITHUB_USER_EMAIL").alert_err(true).await?);
            config.github.user_name =
                Some(env_var("DEPLOY_GITHUB_USER_NAME").alert_err(true).await?);
            config.oss.access_key_id = Some(env_var("OSS_ACCESS_KEY_ID").alert_err(true).await?);
            config.oss.access_key_secret =
                Some(env_var("OSS_ACCESS_KEY_SECRET").alert_err(true).await?);

            tracing::info!("================");
            hugo_deploy(&hugo, &config, true)
                .await
                .alert_err(true)
                .await?;

            tracing::info!("================");
            hugo_deploy(&hugo, &config, false)
                .await
                .alert_err(true)
                .await?;

            let (mb, _) = mp.join_and_get_mb_sample();
            Pushover::new()?
                .send(
                    &format!("Workflow执行成功！\r\n峰值内存：{} MB", mb),
                    PushoverSound::MAGIC,
                )
                .await
        } else {
            Ok(())
        }
    }
}
