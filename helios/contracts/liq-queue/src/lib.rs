#![allow(non_snake_case)]
#![allow(unused_parens)]
#![allow(unused_doc_comments)]
#![allow(non_camel_case_types)]
pub mod contract;
mod error;
pub mod state;
pub mod bid;
pub mod query;

#[cfg(test)]
#[allow(unused_variables)]
mod testing;

pub use crate::error::ContractError;
