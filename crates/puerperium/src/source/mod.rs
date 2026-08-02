//! Where memories come from.
//!
//! The conversion pipeline is source-agnostic by design (see [`crate::convert`]) — it takes
//! materialised [`crate::memory::MemoryRecord`]s and never knows how they arrived. These are
//! the adapters that produce them.
//!
//! Every source is **read-only**. A Cerebro database is another tool's state directory, and
//! usually a live daily driver.

pub mod cerebro_db;
