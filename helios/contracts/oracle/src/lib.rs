#![allow(non_snake_case)]
#![allow(unused_parens)]
#![allow(unused_doc_comments)]
#![allow(non_camel_case_types)]
pub mod contracts;
pub mod state;
pub mod error;
pub mod helpers;

#[cfg(test)]
#[allow(unused_variables)]
pub mod integration_tests;