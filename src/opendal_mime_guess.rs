// Licensed to the Apache Software Foundation (ASF) under one
// or more contributor license agreements.  See the NOTICE file
// distributed with this work for additional information
// regarding copyright ownership.  The ASF licenses this file
// to you under the Apache License, Version 2.0 (the
// "License"); you may not use this file except in compliance
// with the License.  You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing,
// software distributed under the License is distributed on an
// "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied.  See the License for the
// specific language governing permissions and limitations
// under the License.

use opendal::{raw::*, Result};

/// A layer that can automatically set `Content-Type` based on the file extension in the path.
///
/// # MimeGuess
///
/// This layer uses [mime_guess](https://crates.io/crates/mime_guess) to automatically
/// set `Content-Type` based on the file extension in the operation path.
///
/// Specifically, when you call the following methods:
/// - [Operator::write](../struct.Operator.html#method.write)
/// - [Operator::write_with](../struct.Operator.html#method.write_with)
/// - [Operator::writer](../struct.Operator.html#method.writer)
/// - [Operator::writer_with](../struct.Operator.html#method.writer_with)
/// - [Operator::stat](../struct.Operator.html#method.stat)
/// - [Operator::stat_with](../struct.Operator.html#method.stat_with)
/// - [Operator::list_with](../struct.Operator.html#method.list_with)
/// - [Operator::lister_with](../struct.Operator.html#method.lister_with)
/// - [BlockingOperator::write](../struct.BlockingOperator.html#method.write)
/// - [BlockingOperator::write_with](../struct.BlockingOperator.html#method.write_with)
/// - [BlockingOperator::writer](../struct.BlockingOperator.html#method.writer)
/// - [BlockingOperator::writer_with](../struct.BlockingOperator.html#method.writer_with)
/// - [BlockingOperator::stat](../struct.BlockingOperator.html#method.stat)
/// - [BlockingOperator::stat_with](../struct.BlockingOperator.html#method.stat_with)
/// - [BlockingOperator::list_with](../struct.BlockingOperator.html#method.list_with)
/// - [BlockingOperator::lister_with](../struct.BlockingOperator.html#method.lister_with)
///
/// Your operation will automatically carry `Content-Type` information.
///
/// However, please note that this layer will not overwrite the `content_type` you manually set,
/// nor will it overwrite the `content_type` provided by backend services.
///
/// A simple example is that for object storage backends, when you call `stat`, the backend will
/// provide `content_type` information, and `mime_guess` will not be called, but will use
/// the `content_type` provided by the backend.
///
/// But if you use the [Fs](../services/struct.Fs.html) backend to call `stat`, the backend will
/// not provide `content_type` information, and our `mime_guess` will be called to provide you with
/// appropriate `content_type` information.
///
/// Another thing to note is that using this layer does not necessarily mean that the result will 100%
/// contain `content_type` information. If the extension of your path is custom or an uncommon type,
/// the returned result will still not contain `content_type` information (the specific condition here is
/// when [mime_guess::from_path::first_raw](https://docs.rs/mime_guess/latest/mime_guess/struct.MimeGuess.html#method.first_raw)
/// returns `None`).
///
/// # Examples
///
/// ```no_run
/// use anyhow::Result;
/// use opendal::layers::MimeGuessLayer;
/// use opendal::services;
/// use opendal::Operator;
/// use opendal::Scheme;
///
/// let _ = Operator::new(services::Memory::default())
///     .expect("must init")
///     .layer(MimeGuessLayer::default())
///     .finish();
/// ```
#[derive(Debug, Copy, Clone, Default)]
// Developer note:
// The inclusion of a private unit tuple inside the struct here is to force users to
// use `MimeGuessLayer::default()` instead of directly using `MimeGuessLayer` to
// construct instances.
// This way, when we add some optional config methods to this layer in the future,
// the old code can still work perfectly without any breaking changes.
pub struct MimeGuessLayer(());

impl<A: Access> Layer<A> for MimeGuessLayer {
    type LayeredAccess = MimeGuessAccessor<A>;

    fn layer(&self, inner: A) -> Self::LayeredAccess {
        MimeGuessAccessor(inner)
    }
}

#[derive(Clone, Debug)]
pub struct MimeGuessAccessor<A: Access>(A);

fn mime_from_path(path: &str) -> Option<&str> {
    mime_guess::from_path(path).first_raw()
}

fn opwrite_with_mime(path: &str, op: OpWrite) -> OpWrite {
    if op.content_type().is_none() {
        if let Some(mime) = mime_from_path(path) {
            op.with_content_type(mime)
        } else {
            op
        }
    } else {
        op
    }
}

fn rpstat_with_mime(path: &str, rp: RpStat) -> RpStat {
    rp.map_metadata(|metadata| {
        if metadata.content_type().is_none() {
            if let Some(mime) = mime_from_path(path) {
                metadata.with_content_type(mime.into())
            } else {
                metadata
            }
        } else {
            metadata
        }
    })
}

impl<A: Access> LayeredAccess for MimeGuessAccessor<A> {
    type Inner = A;
    type Reader = A::Reader;
    type BlockingReader = A::BlockingReader;
    type Writer = A::Writer;
    type BlockingWriter = A::BlockingWriter;
    type Lister = A::Lister;
    type BlockingLister = A::BlockingLister;

    fn inner(&self) -> &Self::Inner {
        &self.0
    }

    async fn write(&self, path: &str, args: OpWrite) -> Result<(RpWrite, Self::Writer)> {
        self.inner()
            .write(path, opwrite_with_mime(path, args))
            .await
    }

    fn blocking_write(&self, path: &str, args: OpWrite) -> Result<(RpWrite, Self::BlockingWriter)> {
        self.inner()
            .blocking_write(path, opwrite_with_mime(path, args))
    }

    async fn stat(&self, path: &str, args: OpStat) -> Result<RpStat> {
        self.inner()
            .stat(path, args)
            .await
            .map(|rp| rpstat_with_mime(path, rp))
    }

    fn blocking_stat(&self, path: &str, args: OpStat) -> Result<RpStat> {
        self.inner()
            .blocking_stat(path, args)
            .map(|rp| rpstat_with_mime(path, rp))
    }

    async fn read(&self, path: &str, args: OpRead) -> Result<(RpRead, Self::Reader)> {
        self.inner().read(path, args).await
    }

    async fn list(&self, path: &str, args: OpList) -> Result<(RpList, Self::Lister)> {
        self.inner().list(path, args).await
    }

    fn blocking_read(&self, path: &str, args: OpRead) -> Result<(RpRead, Self::BlockingReader)> {
        self.inner().blocking_read(path, args)
    }

    fn blocking_list(&self, path: &str, args: OpList) -> Result<(RpList, Self::BlockingLister)> {
        self.inner().blocking_list(path, args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opendal::{services::Memory, Metakey, Operator};

    const DATA: &str = "<html>test</html>";
    const CUSTOM: &str = "text/custom";
    const HTML: &str = "text/html";

    #[tokio::test]
    async fn test_async() {
        let op_control_group = Operator::new(Memory::default()).unwrap().finish();

        op_control_group.write("test0.html", DATA).await.unwrap();
        assert_eq!(
            op_control_group
                .stat("test0.html")
                .await
                .unwrap()
                .content_type(),
            None
        );

        op_control_group
            .write_with("test1.html", DATA)
            .content_type(CUSTOM)
            .await
            .unwrap();

        assert_eq!(
            op_control_group
                .stat("test1.html")
                .await
                .unwrap()
                .content_type(),
            Some(CUSTOM)
        );

        let op_guess = Operator::new(Memory::default())
            .unwrap()
            .layer(MimeGuessLayer::default())
            .finish();

        op_guess.write("test0.html", DATA).await.unwrap();
        assert_eq!(
            op_guess.stat("test0.html").await.unwrap().content_type(),
            Some(HTML)
        );

        op_guess.write("test1.asdfghjkl", DATA).await.unwrap();
        assert_eq!(
            op_guess
                .stat("test1.asdfghjkl")
                .await
                .unwrap()
                .content_type(),
            None
        );

        op_guess
            .write_with("test2.html", DATA)
            .content_type(CUSTOM)
            .await
            .unwrap();

        assert_eq!(
            op_guess.stat("test2.html").await.unwrap().content_type(),
            Some(CUSTOM)
        );

        let entries = op_guess
            .list_with("")
            .metakey(Metakey::Complete)
            .await
            .unwrap();
        assert_eq!(entries[0].metadata().content_type(), Some(HTML));
        assert_eq!(entries[1].metadata().content_type(), None);
        assert_eq!(entries[2].metadata().content_type(), Some(CUSTOM));
    }

    #[test]
    fn test_blocking() {
        let op_control_group = Operator::new(Memory::default())
            .unwrap()
            .finish()
            .blocking();

        op_control_group.write("test0.html", DATA).unwrap();
        assert_eq!(
            op_control_group.stat("test0.html").unwrap().content_type(),
            None
        );

        op_control_group
            .write_with("test1.html", DATA)
            .content_type(CUSTOM)
            .call()
            .unwrap();

        assert_eq!(
            op_control_group.stat("test1.html").unwrap().content_type(),
            Some(CUSTOM)
        );

        let op_guess = Operator::new(Memory::default())
            .unwrap()
            .layer(MimeGuessLayer::default())
            .finish()
            .blocking();

        op_guess.write("test0.html", DATA).unwrap();
        assert_eq!(
            op_guess.stat("test0.html").unwrap().content_type(),
            Some(HTML)
        );

        op_guess.write("test1.asdfghjkl", DATA).unwrap();
        assert_eq!(
            op_guess.stat("test1.asdfghjkl").unwrap().content_type(),
            None
        );

        op_guess
            .write_with("test2.html", DATA)
            .content_type(CUSTOM)
            .call()
            .unwrap();

        assert_eq!(
            op_guess.stat("test2.html").unwrap().content_type(),
            Some(CUSTOM)
        );

        let entries = op_guess
            .list_with("")
            .metakey(Metakey::Complete)
            .call()
            .unwrap();
        assert_eq!(entries[0].metadata().content_type(), Some(HTML));
        assert_eq!(entries[1].metadata().content_type(), None);
        assert_eq!(entries[2].metadata().content_type(), Some(CUSTOM));
    }
}
