#![doc = include_str!("../README.md")]
#![deny(unsafe_op_in_unsafe_fn)]

#[cfg(test)]
mod test;

mod with_args;
mod without_args;

pub use self::{with_args::CallbackCellArgs, without_args::CallbackCell};
