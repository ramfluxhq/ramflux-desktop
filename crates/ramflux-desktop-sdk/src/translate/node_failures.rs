// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Why a node lookup failed, in the user's terms.
//!
//! On macOS, Local Network privacy stops an app (and the daemon it starts)
//! from reaching hosts on the local network until the user allows it; the
//! connection then fails with `EHOSTUNREACH` (os error 65), while the same
//! host answers other programs. A node add or probe that fails that way is
//! explained as the missing permission, with where to grant it, instead of
//! the bare socket error.
//!
//! The daemon reports a failed discovery request by the HTTP client's top
//! message only (`error sending request for url (...)`), without the socket
//! error under it. When that request went to a host on the local network
//! (a private or link-local IPv4 address, or a `.local` name), the failure
//! is explained as possibly the same permission.

/// Whether a daemon error is an unreachable host (`EHOSTUNREACH`).
pub fn is_host_unreachable(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    // The errno is platform specific: 65 on macOS, 113 on Linux.
    let errno = if cfg!(target_os = "macos") { "os error 65" } else { "os error 113" };
    lower.contains(errno)
        || lower.contains("hostunreachable")
        || lower.contains("host unreachable")
        || lower.contains("no route to host")
        || lower.contains("ehostunreach")
}

/// The host of the URL a failed discovery request names, when it is on the
/// local network.
pub fn failed_local_request(message: &str) -> Option<String> {
    let (_, rest) = message.split_once("error sending request for url (")?;
    let url = rest.split(')').next()?;
    let authority = url.split_once("://").map_or(url, |(_, rest)| rest);
    let host_port = authority.split(['/', '?', '#']).next()?;
    let host = host_port.rsplit_once('@').map_or(host_port, |(_, host)| host);
    let host = host.split(':').next()?.to_ascii_lowercase();
    let local = match host.parse::<std::net::Ipv4Addr>() {
        Ok(ip) => ip.is_private() || ip.is_link_local(),
        Err(_) => host.ends_with(".local"),
    };
    local.then_some(host)
}

/// Whether an unreachable host is most likely the Local Network permission
/// (only macOS asks for one).
pub const fn local_network_permission_applies() -> bool {
    cfg!(target_os = "macos")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unreachable_hosts_are_recognised_in_every_spelling_the_daemon_uses() {
        let errno = if cfg!(target_os = "macos") { 65 } else { 113 };
        assert!(is_host_unreachable(&format!(
            "node error: client record fetch failed: io error: HostUnreachable (os error {errno})"
        )));
        assert!(is_host_unreachable("error sending request: No route to host"));
        assert!(is_host_unreachable("connect: EHOSTUNREACH"));
        assert!(!is_host_unreachable("client record verification failed"));
        assert!(!is_host_unreachable("connection refused (os error 61)"));
    }

    #[test]
    fn a_failed_request_to_a_local_network_host_is_recognised() {
        let failed = |url: &str| {
            failed_local_request(&format!(
                "local bus error: error sending request for url ({url}/.well-known/ramflux/client)"
            ))
        };
        assert_eq!(failed("http://192.168.0.10:8080").as_deref(), Some("192.168.0.10"));
        assert_eq!(failed("https://10.0.0.7").as_deref(), Some("10.0.0.7"));
        assert_eq!(failed("https://node.local").as_deref(), Some("node.local"));
        assert_eq!(failed("https://169.254.1.2:443").as_deref(), Some("169.254.1.2"));
        assert_eq!(failed("http://127.0.0.1:9"), None, "loopback needs no permission");
        assert_eq!(failed("https://node.example.org"), None, "an internet host");
        assert_eq!(failed_local_request("client record verification failed"), None);
    }
}
