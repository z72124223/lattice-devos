//! Sealed raw-duplex boundary between the production Hermes relay and Codex.
//!
//! Implementations remain crate-private so production code cannot accept an
//! arbitrary caller-supplied process or scripted provider. The broker module
//! is the only production implementation; tests may provide an in-memory
//! implementation behind `cfg(test)`.

use std::io::{Read, Write};
use std::sync::Arc;
use std::time::Instant;

use crate::{HermesAdapterError, HermesAdapterErrorKind, HermesAdapterResult};

/// Opens one exact, owned Codex app-server raw stdio session.
pub(crate) trait ProductionCodexProxyProvider: Send {
    /// Returns the process-tree control before this provider is moved into the
    /// host worker. The runner retains a clone so teardown never depends on a
    /// worker blocked in open or pipe I/O.
    fn control(&self) -> Arc<dyn ProductionCodexProxyControl>;

    /// Consumes the sealed provider so one Hermes run can open at most one
    /// app-server session.
    fn open(
        self: Box<Self>,
        absolute_deadline: Instant,
    ) -> HermesAdapterResult<ProductionCodexProxyDuplex>;
}

/// Shared process-tree control retained by the production runner.
///
/// Implementations must make termination idempotent and able to interrupt any
/// blocking raw pipe I/O by killing and reaping the exact owned process tree.
pub(crate) trait ProductionCodexProxyControl: Send + Sync {
    /// Proves the same owned process tree is still live before accepting data.
    fn ensure_running(&self) -> HermesAdapterResult<()>;

    /// Terminates and reaps the complete owned process tree.
    fn terminate(&self) -> HermesAdapterResult<()>;
}

/// Bounded raw app-server stdio. Its process-tree control remains with the
/// production host so dropping or cancelling the host can interrupt blocked
/// I/O without first joining the worker.
pub(crate) struct ProductionCodexProxyDuplex {
    reader: Option<Box<dyn Read + Send>>,
    writer: Option<Box<dyn Write + Send>>,
}

impl ProductionCodexProxyDuplex {
    pub(crate) fn new(reader: Box<dyn Read + Send>, writer: Box<dyn Write + Send>) -> Self {
        Self {
            reader: Some(reader),
            writer: Some(writer),
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
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::sync::{Arc, Mutex};

    use super::*;

    struct ControlProbe(Arc<Mutex<(usize, usize)>>);

    impl ProductionCodexProxyControl for ControlProbe {
        fn ensure_running(&self) -> HermesAdapterResult<()> {
            self.0.lock().expect("probe lock").0 += 1;
            Ok(())
        }

        fn terminate(&self) -> HermesAdapterResult<()> {
            self.0.lock().expect("probe lock").1 += 1;
            Ok(())
        }
    }

    #[test]
    fn duplex_is_send_while_shared_control_remains_outside() {
        fn assert_send<T: Send>() {}
        assert_send::<ProductionCodexProxyDuplex>();
        let probe = Arc::new(Mutex::new((0, 0)));
        let control: Arc<dyn ProductionCodexProxyControl> =
            Arc::new(ControlProbe(Arc::clone(&probe)));
        {
            let mut duplex = ProductionCodexProxyDuplex::new(
                Box::new(Cursor::new(Vec::<u8>::new())),
                Box::new(Vec::<u8>::new()),
            );
            control.ensure_running().expect("lifecycle remains live");
            duplex.close_input();
        }
        assert_eq!(*probe.lock().expect("probe lock"), (1, 0));
        control.terminate().expect("shared lifecycle terminates");
        assert_eq!(*probe.lock().expect("probe lock"), (1, 1));
    }
}
