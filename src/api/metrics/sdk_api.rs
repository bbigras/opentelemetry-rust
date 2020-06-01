//! Metrics SDK API
use crate::api::metrics::{InstrumentKind, Number, NumberKind};
use crate::api::{Context, KeyValue};
use std::fmt;

/// TODO
pub trait MeterCore: fmt::Debug {
    // TODO
    // fn new_async<T, F>(
    //     &self,
    //     name: String,
    //     kind: InstrumentKind,
    //     number: NumberKind,
    //     callback: Runner,
    // ) -> AsyncInstrument
    // where
    //     Self: Sized,
    //     T: Into<String>;

    /// TODO
    fn new_sync_instrument(
        &self,
        name: String,
        instrument_kind: InstrumentKind,
        number_kind: NumberKind,
    ) -> Box<dyn SyncInstrument>;
}

/// TODO
pub trait Instrument: fmt::Debug {
    /// Description of the instrument's descriptor
    fn descriptor(&self) -> &str;
}

/// TODO
pub trait SyncInstrument: fmt::Debug {
    /// TODO
    fn bind<'a>(&self, labels: &'a [KeyValue]) -> Box<dyn BoundSyncInstrument>;

    /// TODO
    fn record_one<'a>(&self, number: Number, labels: &'a [KeyValue]) {
        self.record_one_with_context(&Context::current(), number, labels)
    }

    /// TODO
    fn record_one_with_context<'a>(&self, cx: &Context, number: Number, labels: &'a [KeyValue]);
}

/// TODO
pub trait BoundSyncInstrument: fmt::Debug {
    /// TODO
    fn record_one<'a>(&self, number: Number) {
        self.record_one_with_context(&Context::current(), number)
    }

    /// TODO
    fn record_one_with_context<'a>(&self, cx: &Context, number: Number);
}

/// TODO
pub trait AsyncInstrument: Instrument {}
