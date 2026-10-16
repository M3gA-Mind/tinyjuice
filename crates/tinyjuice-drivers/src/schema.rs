//! What TinyJuice stores, as a schema the host composes.

use tinystoragedrivers_core::schema::{Owner, Schema};

pub fn storage_schema() -> Schema {
    Schema::new(Owner::new("juice").expect("valid owner"))
}

#[cfg(test)]
#[path = "schema_tests.rs"]
mod tests;
