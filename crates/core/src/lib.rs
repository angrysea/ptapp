//! Pure domain logic for the personal training app: no AWS, no I/O.
//! Everything here is unit-tested and shared by the Lambda functions.

pub mod automation;
pub mod bodycomp;
pub mod cardio;
pub mod checkin;
pub mod coach;
pub mod conversations;
pub mod credits;
pub mod habits;
pub mod library;
pub mod model;
pub mod foodimport;
pub mod nutrition;
pub mod progression;
pub mod recipeimport;
pub mod recipes;
pub mod schedule;
pub mod starters;
pub mod sessions;
pub mod records;
pub mod requests;
pub mod stats;
pub mod strength;
pub mod team;
