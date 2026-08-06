//! Session synthesis and in-process e2e harness for the load-perf and fork
//! bench tests.
//!
//! Lives in `intellihelper-shell` (feature `test-support`) rather than
//! `intellihelper-test-support` because synthesis drives the real
//! `JsonlStorageAdapter`; the reverse dependency would be circular.

pub mod e2e;
pub mod synth;
