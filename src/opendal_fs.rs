use opendal::Operator;
use std::{
    io,
    path::{Path, PathBuf},
};
use tokio::{
    fs,
    task::{spawn_blocking, JoinHandle},
};
use walkdir::WalkDir;

pub struct ConcurrentUploadTasks {
    op: Operator,
    handles: Vec<JoinHandle<Result<(), opendal::Error>>>,
}

impl ConcurrentUploadTasks {
    pub fn new(op: Operator) -> Self {
        Self {
            op,
            handles: Vec::new(),
        }
    }

    pub async fn push_single_file(
        &mut self,
        src: impl AsRef<Path>,
        target: &str,
    ) -> Result<(), io::Error> {
        let data = fs::read(src).await?;
        let op = self.op.clone();
        let target = target.to_owned();

        Ok(self.handles.push(tokio::spawn(async move {
            tracing::info!("正在上传：{}", target);
            op.write(&target, data).await
        })))
    }

    pub async fn push_path(&mut self, path: impl AsRef<Path>) -> Result<(), anyhow::Error> {
        let path = path.as_ref();
        Ok(self
            .push_single_file(
                path,
                &path
                    .to_str()
                    .ok_or(anyhow::anyhow!("非法路径！"))?
                    .replace("\\", "/"),
            )
            .await?)
    }

    pub async fn push_str(&mut self, path: &str) -> Result<(), io::Error> {
        self.push_single_file(path, path).await
    }

    pub async fn push_str_seq(&mut self, seq: &Vec<String>) -> Result<(), io::Error> {
        for path in seq {
            self.push_str(path).await?;
        }
        Ok(())
    }

    pub async fn join(self) -> Result<usize, anyhow::Error> {
        let tasks = self.handles.len();
        let mut results = Vec::new();

        for h in self.handles {
            results.push(h.await?);
        }

        for r in results {
            r?;
        }

        Ok(tasks)
    }
}

pub fn collect_files_blocking(dir: impl AsRef<Path>) -> Result<Vec<PathBuf>, anyhow::Error> {
    let mut files = Vec::new();

    for entry in WalkDir::new(dir) {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() {
            files.push(path.to_owned());
        }
    }

    Ok(files)
}

pub async fn collect_files(dir: &str) -> Result<Vec<PathBuf>, anyhow::Error> {
    let dir = dir.to_owned();
    spawn_blocking(move || collect_files_blocking(dir)).await?
}

pub async fn sync_dir(op: &Operator, dir: &str) -> Result<usize, anyhow::Error> {
    tracing::info!("正在加载目录……");
    let files = collect_files(dir).await?;

    tracing::info!("正在删除旧target……");
    op.remove_all(dir).await?;

    tracing::info!("开始上传……");
    let mut upload = ConcurrentUploadTasks::new(op.clone());

    for path in files {
        upload.push_path(path).await?;
    }

    upload.join().await
}
