//! Fake data generators for realistic mock responses
//!
//! This crate provides generators for various types of fake data used in HTTP mocking:
//! - Identifiers (UUIDs, tokens, hashes)
//! - Company data (names, departments, job titles)
//! - Internet data (emails, URLs, IPs, user agents)
//! - Finance data (credit cards, currencies)
//! - Files (PDFs, images, downloads)
//! - Identity data (names, SSNs, demographics)
//! - Contact data (phones, addresses)
//! - Date/time data (timestamps, durations)
//! - Text data (lorem ipsum, descriptions)
//! - Location data (cities, countries, coordinates)
//! - Web data (HTML, JSON, XML responses)
//!
//! `distribution` is the shape a value takes over its support. It lives here
//! rather than beside the entity world because a template calling `fake_*` draws
//! values too, and the world reaches down for its generators rather than the
//! other way round.

// Image generation involves intentional precision loss for graphics operations
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_lossless)]

pub mod company;
pub mod contact;
pub mod datetime;
pub mod distribution;
pub mod files;
pub mod finance;
pub mod identifiers;
pub mod identity;
pub mod internet;
pub mod location;
pub mod place;
pub mod prose;
pub mod rng;
pub mod text;
pub mod web;

// Re-export commonly used functions
pub use company::*;
pub use contact::*;
pub use datetime::*;
pub use files::*;
pub use finance::*;
pub use identifiers::*;
pub use identity::*;
pub use internet::*;
pub use location::*;
pub use place::{Place, place_of, places};
pub use prose::*;
pub use text::*;
pub use web::*;
