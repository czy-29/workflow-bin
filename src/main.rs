use clap::Parser;
use pushover_rs::{send_pushover_request, PushoverSound};
use serde::Deserialize;
use std::{
    env::{self, current_exe},
    time::Duration,
};
use tokio::{fs, process::Command, time::sleep};
use tracing_subscriber::fmt::{format::FmtSpan, time::ChronoLocal};

#[derive(Parser, Debug)]
enum Commands {
    Start,
    UpgradeHugo,
    Run,
}

impl Commands {
    fn trace(self) -> Self {
        match self {
            Self::Start => tracing::info!("workflow-bin start"),
            Self::UpgradeHugo => tracing::info!("workflow-bin upgrade-hugo"),
            Self::Run => tracing::info!("workflow-bin run"),
        }
        self
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

struct Pushover {
    user_key: String,
    app_token: String,
}

impl Pushover {
    fn new() -> Result<Self, anyhow::Error> {
        Ok(Self {
            user_key: env::var("PUSHOVER_USER_KEY")?,
            app_token: env::var("PUSHOVER_APP_TOKEN")?,
        })
    }

    async fn send(&self, message: &str, sound: PushoverSound) -> Result<(), anyhow::Error> {
        tracing::info!("正在发送Pushover消息：{}音色：{}", message, sound);
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
struct WorkflowConfig {
    hugo: HugoConfig,
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

async fn fetch_hugo(config: HugoConfig) -> Result<Command, anyhow::Error> {
    let version = config.version;

    tracing::info!("请求的hugo版本是：{}", version);
    tracing::info!("正在校验现有hugo版本……");

    let hugo = current_exe()?.with_file_name("hugo");
    let mut need_fetch = true;

    if let Ok(output) = Command::new(hugo.clone()).arg("version").output().await {
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
            // __todo__: unzip + save...
        }
    }

    Ok(Command::new(hugo))
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    #[cfg(windows)]
    nu_ansi_term::enable_ansi_support().ok();
    install_tracing();

    let cmd = Commands::parse().trace();

    if cmd.is_start() {
        Pushover::new()?
            .send("Workflow开始执行！", PushoverSound::BIKE)
            .await
    } else {
        let config = WorkflowConfig::read().await?;
        let _hugo = fetch_hugo(config.hugo).await?;

        if cmd.is_run() {
            // __todo__: hugo & deploy
            sleep(Duration::from_secs(10)).await;

            if true {
                Pushover::new()?
                    .send("Workflow执行成功！", PushoverSound::MAGIC)
                    .await
            } else {
                Pushover::new()?
                    .send("Workflow执行失败！", PushoverSound::FALLING)
                    .await
            }
        } else {
            Ok(())
        }
    }
}
