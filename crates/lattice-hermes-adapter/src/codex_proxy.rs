//! Sealed raw-duplex boundary between the production Hermes relay and Codex.
//!
//! Implementations remain crate-private so production code cannot accept an
//! arbitrary caller-supplied process or scripted provider. The broker module
//! is the only production implementation; tests may provide an in-memory
//! implementation behind `cfg(test)`.

use std::io::{Read, Write};
use std::time::Instant;

use crate::{HermesAdapterError, HermesAdapterErrorKind, HermesAdapterResult};

/// Opens one exact, owned Codex app-server raw stdio session.
pub(crate) trait ProductionCodexProxyProvider: Send {
    /// Consumes the sealed provider so one Hermes run can open at most one
    /// app-server session.
    fn open(
        self: Box<Self>,
        absolute_deadline: Instant,
    ) -> HermesAdapterResult<ProductionCodexProxyDuplex>;
}

/// Process-tree ownership retained after the raw stdio handles are split.
pub(crate) trait ProductionCodexProxyLifecycle: Send {
    /// Proves the same owned process tree is still live before accepting data.
    fn ensure_running(&mut self) -> HermesAdapterResult<()>;

    /// Terminates and reaps the complete owned process tree.
    fn terminate(&mut self) -> HermesAdapterResult<()>;
}

/// Bounded raw app-server stdio whose lifecycle is killed and reaped on drop.
pub(crate) struct ProductionCodexProxyDuplex {
    reader: Option<Box<dyn Read + Send>>,
    writer: Option<Box<dyn Write + Send>>,
    lifecycle: Box<dyn ProductionCodexProxyLifecycle>,
    terminated: bool,
}

impl ProductionCodexProxyDuplex {
    pub(crate) fn new(
        reader: Box<dyn Read + Send>,
        writer: Box<dyn Write + Send>,
        lifecycle: Box<dyn ProductionCodexProxyLifecycle>,
    ) -> Self {
        Self {
            reader: Some(reader),
            writer: Some(writer),
            lifecycle,
            terminated: false,
        }
    }

    pub(crate) fn take_reader(&mut self) -> HermesAdapterResult<Box<dyn Read + Send>> {
        self.reader.take().ok_or_else(|| {
            HermesAdapterError::new(
                HermesAdapterErrorKind::Ambiguous,
                "HERMES_CODEX_PROXY_READER_ALREADY_TAKEN",
            )
        })
    }

    pub(crate) fn write_all(&mut self, payload: &[u8]) -> HermesAdapterResult<()> {
        let writer = self.writer.as_mut().ok_or_else(|| {
            HermesAdapterError::new(
                HermesAdapterErrorKind::Failed,
                "HERMES_CODEX_PROXY_INPUT_CLOSED",
            )
        })?;
        writer
            .write_all(payload)
            .and_then(|()| writer.flush())
            .map_err(|_| {
                HermesAdapterError::new(
                    HermesAdapterErrorKind::Transport,
                    "HERMES_CODEX_PROXY_PROVIDER_WRITE_FAILED",
                )
            })
    }

    pub(crate) fn close_input(&mut self) {
        drop(self.writer.take());
    }

    pub(crate) fn ensure_running(&mut self) -> HermesAdapterResult<()> {
        self.lifecycle.ensure_running()
    }

    pub(crate) fn terminate(&mut self) -> HermesAdapterResult<()> {
        self.close_input();
        if self.terminated {
            return Ok(());
        }
        self.lifecycle.terminate()?;
        self.terminated = true;
        Ok(())
    }
}

impl Drop for ProductionCodexProxyDuplex {
    fn drop(&mut self) {
        let _ = self.terminate();
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::sync::{Arc, Mutex};

    use super::*;

    struct LifecycleProbe(Arc<Mutex<(usize, usize)>>);

    impl ProductionCodexProxyLifecycle for LifecycleProbe {
        fn ensure_running(&mut self) -> HermesAdapterResult<()> {
            self.0.lock().expect("probe lock").0 += 1;
            Ok(())
        }

        fn terminate(&mut self) -> HermesAdapterResult<()> {
            self.0.lock().expect("probe lock").1 += 1;
            Ok(())
        }
    }

    #[test]
    fn duplex_is_send_and_kills_lifecycle_on_drop() {
        fn assert_send<T: Send>() {}
        assert_send::<ProductionCodexProxyDuplex>();
        let probe = Arc::new(Mutex::new((0, 0)));
        {
            let mut duplex = ProductionCodexProxyDuplex::new(
                Box::new(Cursor::new(Vec::<u8>::new())),
                Box::new(Vec::<u8>::new()),
                Box::new(LifecycleProbe(Arc::clone(&probe))),
            );
            duplex.ensure_running().expect("lifecycle remains live");
        }
        assert_eq!(*probe.lock().expect("probe lock"), (1, 1));
    }
}
