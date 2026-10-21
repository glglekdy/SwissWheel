//! The in-app updater: check GitHub Releases, download, verify, swap.

pub mod http;
#[cfg(test)]
mod test_server;
