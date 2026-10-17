//! Pure domain logic for the personal training app: no AWS, no I/O.
//! Everything here is unit-tested and shared by the Lambda functions.

pub mod bodycomp;
pub mod library;
pub mod model;
pub mod schedule;
pub mod stats;
