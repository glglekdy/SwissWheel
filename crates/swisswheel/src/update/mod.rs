//! The in-app updater: check GitHub Releases, download, verify, swap.

pub mod http;
pub mod swap;
#[cfg(test)]
mod test_server;
