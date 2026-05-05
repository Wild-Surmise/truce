use crate::events::{EventList, TransportInfo};

pub struct ProcessContext<'a> {
    pub transport: &'a TransportInfo,
    pub sample_rate: f64,
    pub block_size: usize,
    pub output_events: &'a mut EventList,
    params_fn: Option<&'a dyn Fn(u32) -> f64>,
    meters_fn: Option<&'a dyn Fn(u32, f32)>,
}

impl<'a> ProcessContext<'a> {
    pub fn new(
        transport: &'a TransportInfo,
        sample_rate: f64,
        block_size: usize,
        output_events: &'a mut EventList,
    ) -> Self {
        Self {
            transport,
            sample_rate,
            block_size,
            output_events,
            params_fn: None,
            meters_fn: None,
        }
    }

    /// Set the parameter lookup callback.
    #[must_use]
    pub fn with_params(mut self, f: &'a dyn Fn(u32) -> f64) -> Self {
        self.params_fn = Some(f);
        self
    }

    /// Set the meter reporting callback.
    #[must_use]
    pub fn with_meters(mut self, f: &'a dyn Fn(u32, f32)) -> Self {
        self.meters_fn = Some(f);
        self
    }

    /// Read a parameter's plain value by ID.
    ///
    /// Returns `None` when no params callback is wired up (e.g. when a
    /// plugin runs under the bare test driver without a `with_params`
    /// closure). Callers that always run inside a real format wrapper
    /// can `.unwrap_or_default()`. The previous always-`0.0` return
    /// silently masked test-harness misconfiguration as "this param
    /// happens to be at zero", which is indistinguishable from the
    /// host actually setting it to zero.
    #[must_use]
    pub fn param(&self, id: u32) -> Option<f64> {
        self.params_fn.map(|f| f(id))
    }

    /// Report a meter value (0.0 to 1.0).
    pub fn set_meter(&self, id: impl Into<u32>, value: f32) {
        let id = id.into();
        if let Some(f) = self.meters_fn {
            f(id, value);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ProcessStatus {
    /// Plugin produced meaningful output.
    Normal,
    /// Plugin is producing tail. Value = remaining tail samples.
    Tail(u32),
    /// Keep alive even if input is silent.
    KeepAlive,
}
