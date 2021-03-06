//! # OpenTelemetry Tracer interface
//!
//! The OpenTelemetry library achieves in-process context propagation of `Span`s
//! by way of the `Tracer`.
//!
//! The `Tracer` is responsible for tracking the currently active `Span`, and
//! exposes methods for creating and activating new `Spans`. The `Tracer` is
//! configured with `Propagators` which support transferring span context across
//! process boundaries.
//!
//! `Tracer`s are generally expected to be used as singletons. Implementations
//! SHOULD provide a single global default Tracer.
//!
//! Some applications may require multiple `Tracer` instances, e.g. to create
//! `Span`s on behalf of other applications. Implementations MAY provide a
//! global registry of Tracers for such applications.
//!
//! The `Tracer` SHOULD allow end users to configure other tracing components
//! that control how `Span`s are passed across process boundaries, including the
//! binary and text format `Propagator`s used to serialize `Span`s created by
//! the `Tracer`.
//!
//! ## In Synchronous Code
//!
//! Spans can be created and nested manually:
//!
//! ```
//! use opentelemetry::{global, api::{Span, Tracer}};
//! let tracer = global::tracer("my-component");
//!
//! let parent = tracer.start("foo");
//! let child = tracer.span_builder("bar")
//!     .with_parent(parent.span_context())
//!     .start(&tracer);
//!
//! // ...
//!
//! child.end();
//! parent.end();
//! ```
//!
//! Spans can also use the current thread's [`Context`] to track which span is active:
//!
//! ```
//! use opentelemetry::{global, api::{Tracer, SpanKind}};
//! let tracer = global::tracer("my-component");
//!
//! // Create simple spans with `in_span`
//! tracer.in_span("foo", |_foo_cx| {
//!     // parent span is active
//!     tracer.in_span("bar", |_bar_cx| {
//!         // child span is now the active span and associated with the parent span
//!     });
//!     // child has ended, parent now the active span again
//! });
//! // parent has ended, no active spans
//!
//! // -- OR --
//!
//! // create complex spans with span builder and `with_span`
//! let parent_span = tracer.span_builder("foo").with_kind(SpanKind::Server).start(&tracer);
//! tracer.with_span(parent_span, |_foo_cx| {
//!     // parent span is active
//!     let child_span = tracer.span_builder("bar").with_kind(SpanKind::Client).start(&tracer);
//!     tracer.with_span(child_span, |_bar_cx| {
//!         // child span is now the active span and associated with the parent span
//!     });
//!     // child has ended, parent now the active span again
//! });
//! // parent has ended, no active spans
//! ```
//!
//! Spans can also be marked as active, and the resulting guard allows for
//! greater control over when the span is no longer considered active.
//!
//! ```
//! use opentelemetry::{global, api::{Span, Tracer}};
//! let tracer = global::tracer("my-component");
//!
//! let parent_span = tracer.start("foo");
//! let parent_active = tracer.mark_span_as_active(parent_span);
//!
//! {
//!     let child = tracer.start("bar");
//!     let _child_active = tracer.mark_span_as_active(child);
//!
//!     // do work in the context of the child span...
//!
//!     // exiting the scope drops the guard, child is no longer active
//! }
//! // Parent is active span again
//!
//! // Parent can be dropped manually, or allowed to go out of scope as well.
//! drop(parent_active);
//!
//! // no active span
//! ```
//!
//! ## In Asynchronous Code
//!
//! If you are instrumenting code that make use of [`std::future::Future`] or
//! async/await, be sure to use the [`FutureExt`] trait. This is needed because
//! the following example _will not_ work:
//!
//! ```no_run
//! # use opentelemetry::{global, api::Tracer};
//! # let tracer = global::tracer("foo");
//! # let span = tracer.start("foo-span");
//! async {
//!     // Does not work
//!     let _g = tracer.mark_span_as_active(span);
//!     // ...
//! };
//! ```
//!
//! The context guard `_g` will not exit until the future generated by the
//! `async` block is complete. Since futures can be entered and exited
//! _multiple_ times without them completing, the span remains active for as
//! long as the future exists, rather than only when it is polled, leading to
//! very confusing and incorrect output.
//!
//! In order to trace asynchronous code, the [`Future::with_context`] combinator
//! can be used:
//!
//! ```
//! # async fn run() -> Result<(), ()> {
//! use opentelemetry::api::{Context, FutureExt};
//! let cx = Context::current();
//!
//! let my_future = async {
//!     // ...
//! };
//!
//! my_future
//!     .with_context(cx)
//!     .await;
//! # Ok(())
//! # }
//! ```
//!
//! [`Future::with_context`] attaches a context to the future, ensuring that the
//! context's lifetime is as long as the future's.
//!
//! [`std::future::Future`]: https://doc.rust-lang.org/stable/std/future/trait.Future.html
//! [`FutureExt`]: ../futures/trait.FutureExt.html
//! [`Future::with_context`]: ../futures/trait.FutureExt.html#method.with_context
//! [`Context`]: ../../context/struct.Context.html
use crate::api::{
    self,
    context::{Context, ContextGuard},
    TraceContextExt,
};
use std::fmt;
use std::time::SystemTime;

/// Interface for constructing `Span`s.
pub trait Tracer: fmt::Debug + 'static {
    /// The `Span` type used by this `Tracer`.
    type Span: api::Span;

    /// Returns a span with an invalid `SpanContext`. Used by functions that
    /// need to return a default span like `get_active_span` if no span is present.
    fn invalid(&self) -> Self::Span;

    /// Starts a new `Span`.
    ///
    /// By default the currently active `Span` is set as the new `Span`'s
    /// parent. The `Tracer` MAY provide other default options for newly
    /// created `Span`s.
    ///
    /// `Span` creation MUST NOT set the newly created `Span` as the currently
    /// active `Span` by default, but this functionality MAY be offered additionally
    /// as a separate operation.
    ///
    /// Each span has zero or one parent spans and zero or more child spans, which
    /// represent causally related operations. A tree of related spans comprises a
    /// trace. A span is said to be a _root span_ if it does not have a parent. Each
    /// trace includes a single root span, which is the shared ancestor of all other
    /// spans in the trace. Implementations MUST provide an option to create a `Span` as
    /// a root span, and MUST generate a new `TraceId` for each root span created.
    /// For a Span with a parent, the `TraceId` MUST be the same as the parent.
    /// Also, the child span MUST inherit all `TraceState` values of its parent by default.
    ///
    /// A `Span` is said to have a _remote parent_ if it is the child of a `Span`
    /// created in another process. Each propagators' deserialization must set
    /// `is_remote` to true on a parent `SpanContext` so `Span` creation knows if the
    /// parent is remote.
    fn start(&self, name: &str) -> Self::Span {
        self.start_from_context(name, &Context::current())
    }

    /// Starts a new `Span` in a given context
    ///
    /// By default the currently active `Span` is set as the new `Span`'s
    /// parent. The `Tracer` MAY provide other default options for newly
    /// created `Span`s.
    ///
    /// `Span` creation MUST NOT set the newly created `Span` as the currently
    /// active `Span` by default, but this functionality MAY be offered additionally
    /// as a separate operation.
    ///
    /// Each span has zero or one parent spans and zero or more child spans, which
    /// represent causally related operations. A tree of related spans comprises a
    /// trace. A span is said to be a _root span_ if it does not have a parent. Each
    /// trace includes a single root span, which is the shared ancestor of all other
    /// spans in the trace. Implementations MUST provide an option to create a `Span` as
    /// a root span, and MUST generate a new `TraceId` for each root span created.
    /// For a Span with a parent, the `TraceId` MUST be the same as the parent.
    /// Also, the child span MUST inherit all `TraceState` values of its parent by default.
    ///
    /// A `Span` is said to have a _remote parent_ if it is the child of a `Span`
    /// created in another process. Each propagators' deserialization must set
    /// `is_remote` to true on a parent `SpanContext` so `Span` creation knows if the
    /// parent is remote.
    fn start_from_context(&self, name: &str, context: &Context) -> Self::Span;

    /// Creates a span builder
    ///
    /// An ergonomic way for attributes to be configured before the `Span` is started.
    fn span_builder(&self, name: &str) -> SpanBuilder;

    /// Create a span from a `SpanBuilder`
    fn build(&self, builder: SpanBuilder) -> Self::Span {
        self.build_with_context(builder, &Context::current())
    }

    /// Create a span from a `SpanBuilder`
    fn build_with_context(&self, builder: SpanBuilder, cx: &Context) -> Self::Span;

    /// Mark a given `Span` as active.
    ///
    /// The `Tracer` MUST provide a way to update its active `Span`, and MAY provide convenience
    /// methods to manage a `Span`'s lifetime and the scope in which a `Span` is active. When an
    /// active `Span` is made inactive, the previously-active `Span` SHOULD be made active. A `Span`
    /// maybe finished (i.e. have a non-null end time) but still be active. A `Span` may be active
    /// on one thread after it has been made inactive on another.
    ///
    /// # Examples
    ///
    /// ```
    /// use opentelemetry::{global, api::{Span, Tracer, KeyValue}};
    ///
    /// fn my_function() {
    ///     let tracer = global::tracer("my-component-a");
    ///     // start an active span in one function
    ///     let span = tracer.start("span-name");
    ///     let _guard = tracer.mark_span_as_active(span);
    ///     // anything happening in functions we call can still access the active span...
    ///     my_other_function();
    /// }
    ///
    /// fn my_other_function() {
    ///     // call methods on the current span from
    ///     global::tracer("my-component-b").get_active_span(|span| {
    ///         span.add_event("An event!".to_string(), vec![KeyValue::new("happened", true)]);
    ///     });
    /// }
    /// ```
    #[must_use = "Dropping the guard detaches the context."]
    fn mark_span_as_active(&self, span: Self::Span) -> ContextGuard {
        let cx = Context::current_with_span(span);
        cx.attach()
    }

    /// Executes a closure with a reference to this thread's current span.
    ///
    /// # Examples
    ///
    /// ```
    /// use opentelemetry::{global, api::{Span, Tracer, KeyValue}};
    ///
    /// fn my_function() {
    ///     // start an active span in one function
    ///     global::tracer("my-component").in_span("span-name", |_cx| {
    ///         // anything happening in functions we call can still access the active span...
    ///         my_other_function();
    ///     })
    /// }
    ///
    /// fn my_other_function() {
    ///     // call methods on the current span from
    ///     global::tracer("my-component").get_active_span(|span| {
    ///         span.add_event("An event!".to_string(), vec![KeyValue::new("happened", true)]);
    ///     })
    /// }
    /// ```
    fn get_active_span<F, T>(&self, f: F) -> T
    where
        F: FnOnce(&dyn api::Span) -> T,
        Self: Sized,
    {
        f(Context::current().span())
    }

    /// Start a new span and execute the given closure with reference to the span's
    /// context.
    ///
    /// This method starts a new span and sets it as the active span for the given
    /// function. It then executes the body. It closes the span before returning the
    /// execution result.
    ///
    /// # Examples
    ///
    /// ```
    /// use opentelemetry::{global, api::{Span, Tracer, KeyValue}};
    ///
    /// fn my_function() {
    ///     // start an active span in one function
    ///     global::tracer("my-component").in_span("span-name", |_cx| {
    ///         // anything happening in functions we call can still access the active span...
    ///         my_other_function();
    ///     })
    /// }
    ///
    /// fn my_other_function() {
    ///     // call methods on the current span from
    ///     global::tracer("my-component").get_active_span(|span| {
    ///         span.add_event("An event!".to_string(), vec![KeyValue::new("happened", true)]);
    ///     })
    /// }
    /// ```
    fn in_span<T, F>(&self, name: &'static str, f: F) -> T
    where
        F: FnOnce(Context) -> T,
        Self::Span: Send + Sync,
    {
        let span = self.start(name);
        let cx = Context::current_with_span(span);
        let _guard = cx.clone().attach();
        f(cx)
    }

    /// Start a new span and execute the given closure with reference to the span's
    /// context.
    ///
    /// This method starts a new span and sets it as the active span for the given
    /// function. It then executes the body. It closes the span before returning the
    /// execution result.
    ///
    /// # Examples
    ///
    /// ```
    /// use opentelemetry::{global, api::{Span, SpanKind, Tracer, KeyValue}};
    ///
    /// fn my_function() {
    ///     let tracer = global::tracer("my-component");
    ///     // start a span with custom attributes via span bulder
    ///     let span = tracer.span_builder("span-name").with_kind(SpanKind::Server).start(&tracer);
    ///     // Mark the span as active for the duration of the closure
    ///     global::tracer("my-component").with_span(span, |_cx| {
    ///         // anything happening in functions we call can still access the active span...
    ///         my_other_function();
    ///     })
    /// }
    ///
    /// fn my_other_function() {
    ///     // call methods on the current span from
    ///     global::tracer("my-component").get_active_span(|span| {
    ///         span.add_event("An event!".to_string(), vec![KeyValue::new("happened", true)]);
    ///     })
    /// }
    /// ```
    fn with_span<T, F>(&self, span: Self::Span, f: F) -> T
    where
        F: FnOnce(Context) -> T,
        Self::Span: Send + Sync,
    {
        let cx = Context::current_with_span(span);
        let _guard = cx.clone().attach();
        f(cx)
    }
}

/// `SpanBuilder` allows span attributes to be configured before the span
/// has started.
///
/// ```rust
/// use opentelemetry::{
///     api::{Provider, SpanBuilder, SpanKind, Tracer},
///     global,
/// };
///
/// let tracer = global::tracer("example-tracer");
///
/// // The builder can be used to create a span directly with the tracer
/// let _span = tracer.build(SpanBuilder {
///     name: "example-span-name".to_string(),
///     span_kind: Some(SpanKind::Server),
///     ..Default::default()
/// });
///
/// // Or used with builder pattern
/// let _span = tracer
///     .span_builder("example-span-name")
///     .with_kind(SpanKind::Server)
///     .start(&tracer);
/// ```
#[derive(Clone, Debug, Default)]
pub struct SpanBuilder {
    /// Parent `SpanContext`
    pub parent_context: Option<api::SpanContext>,
    /// Trace id, useful for integrations with external tracing systems.
    pub trace_id: Option<api::TraceId>,
    /// Span id, useful for integrations with external tracing systems.
    pub span_id: Option<api::SpanId>,
    /// Span kind
    pub span_kind: Option<api::SpanKind>,
    /// Span name
    pub name: String,
    /// Span start time
    pub start_time: Option<SystemTime>,
    /// Span end time
    pub end_time: Option<SystemTime>,
    /// Span attributes
    pub attributes: Option<Vec<api::KeyValue>>,
    /// Span Message events
    pub message_events: Option<Vec<api::Event>>,
    /// Span Links
    pub links: Option<Vec<api::Link>>,
    /// Span status code
    pub status_code: Option<api::StatusCode>,
    /// Span status message
    pub status_message: Option<String>,
    /// Sampling result
    pub sampling_result: Option<api::SamplingResult>,
}

/// SpanBuilder methods
impl SpanBuilder {
    /// Create a new span builder from a span name
    pub fn from_name(name: String) -> Self {
        SpanBuilder {
            parent_context: None,
            trace_id: None,
            span_id: None,
            span_kind: None,
            name,
            start_time: None,
            end_time: None,
            attributes: None,
            message_events: None,
            links: None,
            status_code: None,
            status_message: None,
            sampling_result: None,
        }
    }

    /// Assign parent context
    pub fn with_parent(self, parent_context: api::SpanContext) -> Self {
        SpanBuilder {
            parent_context: Some(parent_context),
            ..self
        }
    }

    /// Specify trace id to use if no parent context exists
    pub fn with_trace_id(self, trace_id: api::TraceId) -> Self {
        SpanBuilder {
            trace_id: Some(trace_id),
            ..self
        }
    }

    /// Assign span id
    pub fn with_span_id(self, span_id: api::SpanId) -> Self {
        SpanBuilder {
            span_id: Some(span_id),
            ..self
        }
    }

    /// Assign span kind
    pub fn with_kind(self, span_kind: api::SpanKind) -> Self {
        SpanBuilder {
            span_kind: Some(span_kind),
            ..self
        }
    }

    /// Assign span start time
    pub fn with_start_time<T: Into<SystemTime>>(self, start_time: T) -> Self {
        SpanBuilder {
            start_time: Some(start_time.into()),
            ..self
        }
    }

    /// Assign span end time
    pub fn with_end_time<T: Into<SystemTime>>(self, end_time: T) -> Self {
        SpanBuilder {
            end_time: Some(end_time.into()),
            ..self
        }
    }

    /// Assign span attributes
    pub fn with_attributes(self, attributes: Vec<api::KeyValue>) -> Self {
        SpanBuilder {
            attributes: Some(attributes),
            ..self
        }
    }

    /// Assign message events
    pub fn with_message_events(self, message_events: Vec<api::Event>) -> Self {
        SpanBuilder {
            message_events: Some(message_events),
            ..self
        }
    }

    /// Assign links
    pub fn with_links(self, links: Vec<api::Link>) -> Self {
        SpanBuilder {
            links: Some(links),
            ..self
        }
    }

    /// Assign status code
    pub fn with_status_code(self, code: api::StatusCode) -> Self {
        SpanBuilder {
            status_code: Some(code),
            ..self
        }
    }

    /// Assign status message
    pub fn with_status_message(self, message: String) -> Self {
        SpanBuilder {
            status_message: Some(message),
            ..self
        }
    }

    /// Assign sampling result
    pub fn with_sampling_result(self, sampling_result: api::SamplingResult) -> Self {
        SpanBuilder {
            sampling_result: Some(sampling_result),
            ..self
        }
    }

    /// Builds a span with the given tracer from this configuration.
    pub fn start<T: api::Tracer>(self, tracer: &T) -> T::Span {
        tracer.build(self)
    }
}
