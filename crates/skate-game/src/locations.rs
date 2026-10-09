//! Native location loading and travel.
mod input;
mod markers;
mod runtime;

mod catalog;
mod model;

mod engine;
pub(crate) use engine::{
    LocationPlugin, LocationRegistry, approval, blocked, catalogs, clear, interior_label, load,
    load_status, retain_admitted, retire, set, status,
};
