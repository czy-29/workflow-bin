use opendal::Operator;
use std::path::Path;
use tokio::fs;

pub async fn upload_file(
    op: &Operator,
    src: impl AsRef<Path>,
    target: &str,
) -> Result<(), anyhow::Error> {
    tracing::info!("正在上传：{}", target);
    Ok(op.write(target, fs::read(src).await?).await?)
}
