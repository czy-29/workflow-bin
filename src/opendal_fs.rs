use opendal::Operator;
use std::{io, path::Path};
use tokio::{fs, task::JoinHandle};

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

    pub async fn push_str(&mut self, path: &str) -> Result<(), io::Error> {
        self.push_single_file(path, path).await
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
